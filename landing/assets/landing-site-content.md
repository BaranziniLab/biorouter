# Biorouter Landing Site — Content & Requirements Tracker

## Overview
Landing site for UCSF Biorouter. Design: Material Design with OpenAI aesthetics — minimalist, dark theme, mobile/tablet/desktop responsive.

**Color palette:** UCSF teal (#18A3AC), UCSF navy (#052049), dark background (#0a0a0a)
**Font:** Inter (body), JetBrains Mono (code)
**Version:** v1.91.2 (September 2026)

---

## File Structure

```
Biorouter Landing/
├── index.html          # Main entry — redirects to intro.html
├── intro.html          # Tab 1: Introduction
├── download.html       # Tab 2: Download
├── docs.html           # Tab 3: Documentation
├── baam.html           # Tab 4: BAAM (Agent Marketplace)
├── about.html          # Tab 5: About
├── shared.css          # Shared styles across all pages
├── icon.png            # Biorouter logo (copied from ui/desktop/src/images/)
└── landing-site-content.md  # This file
```

Source icon: `/Users/wgu/Desktop/biorouter/ui/desktop/src/images/icon.png`
Source docs: `/Users/wgu/Desktop/biorouter/documentation/`
GitHub: https://github.com/BaranziniLab/biorouter

---

## Tab 1 — Introduction (`intro.html`)

### Banner / Hero
- Biorouter logo (icon.png) with glow animation
- Version badge: "v1.91.2 Now Available"
- Headline: "UCSF Biorouter"
- Subheading: AI-powered integrated research environment tagline
- CTA buttons: Download, Documentation, GitHub

### Summary (from presentation)
**Core message:** Biorouter is an Agentic-AI-powered Integrated Biomedical Research Environment.

**Problem statement:**
- Biomedical researchers face: heavy local compute needs, unique local setups, proprietary codebases, on-prem datasets, sensitive patient/research data
- Commercial AI services are limited due to privacy/security
- UCSF enterprise AI lacks agentic support

**Solution — Biorouter is an all-in-one tool with:**
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

**Key capability — Share Workflow and Skills:**
- Can be shared across institutions
- Standardized analysis pipelines with AI-driven adaptation

**Key capability — Diverse Research Database Connections:**
- Structured and unstructured electronic health records
- Biological knowledge graph (SPOKE)

**Conclusion — Biorouter:**
- Unifies the full research stack (commercial + institution-hosted + local AI models)
- Is secure by design: enterprise-grade AI security
- Enables agentic orchestration and automation
- Is federated-collaboration-ready
- Offers growing ecosystem (skills, agents, databases, etc.)
- Ongoing: UCSF Institution-managed AI Agent Marketplace (quality-controlled, community-driven)

### Feature Cards (6)
1. Enterprise-Grade Security — UCSF-hosted AI, local Ollama, patient data stays in institution
2. Agentic Orchestration — MCP agents, clinical DBs, knowledge graphs
3. Federated Collaboration — Workflows shareable across institutions without sharing data
4. Multi-Provider LLMs — Claude, GPT, Gemini, commercial/institutional/local
5. Automation & Scheduling — Workflows, cron scheduling
6. Desktop + CLI — GUI for exploration, CLI for scripting

### Provider Ecosystem
Chips for: UCSF Azure OpenAI (highlight), UCSF Amazon Bedrock (highlight), Anthropic Claude, OpenAI GPT, Google Gemini, Ollama (Local), OpenRouter, X.AI Grok, Custom Endpoints

### Bottom Links
- About & Acknowledgements (→ about.html)
- v1.91.2 Release Notes (https://github.com/BaranziniLab/biorouter/releases/tag/v1.91.2)
- Baranzini Lab (https://baranzinilab.ucsf.edu/)

---

## Tab 2 — Download (`download.html`)

### Hero
- Version badge: "v1.91.2 · September 2026"
- Title: "Download Biorouter"
- Subtitle: "Native installers for every major platform. Open source and free."

### Smart OS Detection
JavaScript detects the visitor's OS and shows the recommended download button prominently.

**Detection logic:**
- macOS Apple Silicon (arm) → arm64.dmg
- macOS Intel → x64.dmg
- Windows → win32-x64.zip
- Linux → amd64.deb (default for Linux)
- Unknown/iOS/Android → show all options without a primary

### Download Links (v1.91.2)
| Platform | File | URL |
|---|---|---|
| macOS Apple Silicon | Biorouter-1.91.2-arm64.dmg | https://github.com/BaranziniLab/biorouter/releases/download/v1.91.2/Biorouter-1.91.2-arm64.dmg |
| macOS Intel | Biorouter-1.91.2-x64.dmg | https://github.com/BaranziniLab/biorouter/releases/download/v1.91.2/Biorouter-1.91.2-x64.dmg |
| Windows x64 | Biorouter-win32-x64-1.91.2.zip | https://github.com/BaranziniLab/biorouter/releases/download/v1.91.2/Biorouter-win32-x64-1.91.2.zip |
| Linux Ubuntu/Pop!_OS | biorouter_1.91.2_amd64.deb | https://github.com/BaranziniLab/biorouter/releases/download/v1.91.2/biorouter_1.91.2_amd64.deb |
| Linux Fedora/RHEL | Biorouter-1.91.2-1.x86_64.rpm | https://github.com/BaranziniLab/biorouter/releases/download/v1.91.2/Biorouter-1.91.2-1.x86_64.rpm |
| Linux CLI Debian/Ubuntu | biorouter-cli_1.91.2_amd64.deb | https://github.com/BaranziniLab/biorouter/releases/download/v1.91.2/biorouter-cli_1.91.2_amd64.deb |
| Linux CLI Fedora/RHEL | biorouter-cli-1.91.2-1.x86_64.rpm | https://github.com/BaranziniLab/biorouter/releases/download/v1.91.2/biorouter-cli-1.91.2-1.x86_64.rpm |

### Install Commands
- macOS: Open DMG and drag Biorouter.app to /Applications
- Windows: Unzip and run Biorouter.exe
- Linux Debian: sudo apt install ./biorouter_1.91.2_amd64.deb
- Linux RPM: sudo dnf install ./Biorouter-1.91.2-1.x86_64.rpm

### Setup Note / Info Box
After installing, users should:
1. Set up UCSF Versa API (UCSF affiliated) → https://ai.ucsf.edu/platforms-tools-and-resources/ucsf-versa
2. For other institutions: check institution policies for LLM hosting
3. For local models: install Ollama → https://ollama.com/download
4. Alternatively: commercial API keys — Anthropic (https://platform.claude.com/) or OpenAI (https://openai.com/api/) — but per institution policy, access to research/proprietary DBs may be limited
5. Link to Documentation Setup Guide tab

---

## Tab 3 — Documentation (`docs.html`)

### Left Sidebar Navigation
**Getting Started:**
- Installation & Setup
- UCSF Setup Guide

**Core Concepts:**
- Providers & Models
- Extensions & Skills
- Workflows & Automation
- Data Privacy

**Advanced:**
- Architecture
- MCP Agents

### Content Pages

#### Installation & Setup
Source: `/Users/wgu/Desktop/biorouter/documentation/installation-setup.md`
- Step 1: Download (macOS DMG, Windows ZIP, Linux DEB/RPM, macOS CLI)
- Step 2: Configure LLM Provider (UCSF Azure, Bedrock, Ollama, commercial keys)
- Step 3: Verify setup
- Step 4: Enable extensions
- Step 5: MCP Agents
- Config file location: `~/.config/biorouter/config.yaml`
- Troubleshooting table

#### UCSF Setup Guide
- Option A: UCSF Versa → https://ai.ucsf.edu/platforms-tools-and-resources/ucsf-versa
- Option B: Azure OpenAI (UCSF ChatGPT)
- Option C: Amazon Bedrock (UCSF-hosted Anthropic) — requires aws sso login
- Option D: Local Ollama (air-gapped)
- Other institutions guidance
- Warning about PHI/patient data

#### Providers & Models
Source: `/Users/wgu/Desktop/biorouter/documentation/providers-and-models.md`
- UCSF institutional providers table
- Commercial providers table with env vars and key URLs
- Local models (Ollama) with example commands

#### Extensions & Skills
Source: `/Users/wgu/Desktop/biorouter/documentation/extensions-skills-mcp.md`
- Built-in extensions table (Developer, Biorouter Copilot, Memory, Auto Visualiser, Chat Recall, Code Execution)
- Adding external MCP servers (Desktop UI + config YAML)
- Skills system description

#### Workflows & Automation
Source: `/Users/wgu/Desktop/biorouter/documentation/workflows.md`
- What is a Workflow?
- Workflow YAML structure example
- Running workflows (Desktop + CLI)
- Scheduling with cron
- Sharing workflows across institutions

#### Data Privacy
Source: `/Users/wgu/Desktop/biorouter/documentation/data-privacy.md`
- Warning about patient data / PHI
- Provider privacy properties table
- Best practices list

#### Architecture
Source: `/Users/wgu/Desktop/biorouter/documentation/architecture.md`
- Three-layer architecture (Interface, Agent, Extensions)
- Backend: Rust workspace, crate table
- Frontend: Electron + React
- MCP protocol overview

#### MCP Agents
- How to add agents via Desktop UI
- How to add via config YAML
- Remote HTTP agents
- Prerequisites (uvx / npx)
- Link to BAAM marketplace

---

## Tab 4 — BAAM (`baam.html`)

**Full name:** UCSF Biorouter AI Agent Marketplace (UCSF BAAM)

### Hero
- BAAM acronym badge
- Title: "Biorouter AI Agent Marketplace"
- Description: curated, quality-controlled ecosystem for biomedical research
- Search box (filters agent cards in real-time)

### Agent Cards
Design reference: https://block.github.io/goose/extensions/ — but more aesthetic, cohesive with landing design.

Each card contains:
- Agent icon (emoji)
- Agent name
- Organization
- GitHub link (icon button)
- Description
- One-liner install command (copyable)
- Tags (UCSF, MCP, category)

#### Agent 1 — UCSF OMOP Agent
- **Name:** UCSF OMOP Agent
- **Org:** BaranziniLab / UCSF
- **GitHub:** https://github.com/BaranziniLab/UCSFOMOPAgent
- **Description:** MCP server enabling SQL queries on UCSF OMOP de-identified clinical database. Read-only access to standardized EHRs under OMOP Common Data Model.
- **Command:** `uvx --from git+https://github.com/BaranziniLab/UCSFOMOPAgent ucsfomopagent`
- **Tags:** UCSF, MCP, Clinical, OMOP

#### Agent 2 — SPOKEAgent
- **Name:** SPOKEAgent
- **Org:** BaranziniLab / UCSF
- **GitHub:** https://github.com/BaranziniLab/SPOKEAgent
- **Description:** MCP server for querying SPOKE biomedical knowledge graph. Cypher queries against large-scale biomedical DB for rapid biomedical knowledge inference.
- **Command:** `uvx --from git+https://github.com/BaranziniLab/SPOKEAgent spokeagent`
- **Tags:** UCSF, MCP, Knowledge Graph, SPOKE

#### Agent 3 — MedCP
- **Name:** MedCP
- **Org:** BaranziniLab / UCSF
- **GitHub:** https://github.com/BaranziniLab/MedCP
- **Description:** Medical Model Context Protocol. Transforms Biorouter into a medical AI assistant with secure, local access to EHRs and biomedical knowledge graphs.
- **Command:** `uvx --from git+https://github.com/BaranziniLab/MedCP medcp`
- **Tags:** UCSF, MCP, Clinical AI, Local

#### Agent 4 — Playwright MCP
- **Name:** Playwright MCP
- **Org:** Microsoft
- **GitHub:** https://github.com/microsoft/playwright-mcp
- **Description:** MCP server enabling LLMs to interact with web pages via accessibility snapshots. Browser automation without vision models.
- **Command:** `npx @playwright/mcp@latest`
- **Tags:** MCP, Browser, Web, Automation

### Bottom CTA
"Have an agent to contribute? Submit it to the marketplace." → GitHub link

---

## Tab 5 — About (`about.html`)

### News
1. **Biorouter v1.91.2 Release** (September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.91.2
   - What's new: Copilot status appears only in conversations that request it and can be dismissed; prompts and slash commands use its current name; Windows child processes request hidden console windows; chat gains quoted selections, persistent drafts, active extensions in the slash picker, and adjustable text sizes

2. **Biorouter v1.91.1 Release** (September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.91.1
   - What's new: on Windows, Biorouter's network sockets no longer leak into the programs it starts, so ports are released and the tunnel closes cleanly; on macOS, inline-Python extensions no longer occasionally fail to start; and the test suite's flaky failures are fixed at their causes

3. **Biorouter v1.91.0 Release** (September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.91.0
   - What's new: Biorouter Copilot observes and controls the desktop with an approval for every request, web and documents become their own capability, office contexts load only when needed, Windows gets its first installer and a long list of fixes, and third-party attribution is now correct

4. **Biorouter v1.90.5 Release** (September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.90.5
   - What's new: chats keep their place when a tab closes mid-reply, streamed text keeps repeated tokens at chunk boundaries, mid-turn steering stays queued until the running turn consumes it, the artifact preview adapts between a side panel and a vertical layout, and scheduled work follows the caller's reach

5. **Biorouter v1.90.0 Release** (September 2026)
   - Link: https://github.com/BaranziniLab/biorouter/releases/tag/v1.90.0
   - What's new: every tool driven across providers, and a review run before the drive

6. **UCSF AI Research Day** (2026)
   - Link: https://ai.ucsf.edu/researchday
   - Biorouter to be featured

### Resource Links
- GitHub: https://github.com/BaranziniLab/biorouter
- Baranzini Lab: https://baranzinilab.ucsf.edu/
- Bakar CHSI: https://bakarinstitute.ucsf.edu/
- UCSF ARS: https://ars.ucsf.edu/
- UCSF Versa: https://ai.ucsf.edu/platforms-tools-and-resources/ucsf-versa
- UCSF AI Research Day: https://ai.ucsf.edu/researchday

### Acknowledgements
**The Baranzini Lab (https://baranzinilab.ucsf.edu/)**
- Gianmarco Bellucci
- Sergio Baranzini

**Bakar Computational Health Science Institute (BCHSI) (https://bakarinstitute.ucsf.edu/)**
- Sharat Israni
- Marina Sirota

**UCSF Academic Research Services (ARS) (https://ars.ucsf.edu/)**
- William Santo
- Evan Philps
- Rick Larson
- Oksana Gologorskaya

### About the Developer
- **Name:** Wanjun Gu
- **Role:** Lead Developer, Baranzini Lab, UCSF
- **Email:** wanjun.gu@ucsf.edu
- **GitHub:** https://github.com/Broccolito
- **Lab:** https://baranzinilab.ucsf.edu/

---

## Design Requirements

### General
- [x] Material Design with OpenAI aesthetics
- [x] Dark theme (#0a0a0a background)
- [x] UCSF teal (#18A3AC) as accent color
- [x] Minimalist, clean, lots of whitespace
- [x] Mobile/tablet/desktop responsive
- [x] Biorouter logo in banner (with glow/shadow)
- [x] No emoji unless UI icons

### Navigation
- [x] Fixed top navbar
- [x] Active tab indicator
- [x] Mobile hamburger menu
- [x] Glassmorphism navbar (backdrop blur)

### Introduction Tab
- [x] Animated hero banner
- [x] Feature cards (6)
- [x] Provider ecosystem chips
- [x] Links to About, Release Notes, Baranzini Lab

### Download Tab
- [x] Smart OS detection → primary download button
- [x] Complete platform table with all download links
- [x] Install instructions per platform
- [x] Setup guide link
- [x] Info box about UCSF Versa, Ollama, commercial APIs

### Documentation Tab
- [x] Sidebar navigation
- [x] Multiple doc pages (8 total)
- [x] UCSF-specific setup
- [x] Code blocks with proper monospace font
- [x] Data tables
- [x] Warning/info alert boxes

### BAAM Tab
- [x] Search functionality (real-time filter)
- [x] Agent cards with: name, org, description, command, GitHub link, tags
- [x] Copy-to-clipboard for commands
- [x] 4 agents: UCSFOMOPAgent, SPOKEAgent, MedCP, Playwright MCP

### About Tab
- [x] News cards with dates
- [x] Resource link grid
- [x] Acknowledgements with group labels
- [x] Developer card with contact info

---

## Source Files Referenced
- `/Users/wgu/Desktop/biorouter/documentation/installation-setup.md`
- `/Users/wgu/Desktop/biorouter/documentation/data-privacy.md`
- `/Users/wgu/Desktop/biorouter/documentation/providers-and-models.md`
- `/Users/wgu/Desktop/biorouter/documentation/extensions-skills-mcp.md`
- `/Users/wgu/Desktop/biorouter/documentation/workflows.md`
- `/Users/wgu/Desktop/biorouter/documentation/schedulers.md`
- `/Users/wgu/Desktop/biorouter/documentation/architecture.md`
- `/Users/wgu/Desktop/biorouter/ui/desktop/src/images/icon.png`
- `/Users/wgu/Desktop/biorouter/README.md`
- `/Users/wgu/Desktop/biorouter/docs/release-notes/vX.Y.Z.md`
- GitHub: https://github.com/BaranziniLab/biorouter
- GitHub Agents: UCSFOMOPAgent, SPOKEAgent, MedCP, playwright-mcp
