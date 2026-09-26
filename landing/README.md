# Biorouter Landing Site

Static landing site for **Biorouter v1.88.1** — a local-first AI routing layer for clinical research developed at the [Baranzini Lab](https://baranzinilab.ucsf.edu/), UCSF.

**Live site:** <https://biorouter.ucsf.edu/>

---

## Pages

| File | Tab | Description |
| ---- | --- | ----------- |
| `index.html` | — | Redirects to `intro.html` |
| `intro.html` | Introduction | Hero section, feature highlights, supported AI providers |
| `download.html` | Download | OS-aware download card (auto-detects macOS arm64/x64, Windows, Linux), platform table, setup instructions |
| `docs.html` | Documentation | Sidebar-navigated documentation covering installation, configuration, workflows, federation, and CLI reference |
| `baam.html` | BAAM Marketplace | Biorouter AI Agent Marketplace — agent cards, install instructions, and community workflows |
| `about.html` | About | News & announcements, related links, acknowledgments, developer info |

### Shared assets

- `shared.css` — design tokens, navbar, buttons, tables, and all shared component styles
- `icon.png` — Biorouter app icon
- `assets/ehr-diabetes-recipe.yaml` — downloadable example workflow (EHR Diabetes Demographics Dashboard)
- `assets/landing-site-content.md` — content requirements tracker

---

## Design System

- **Theme:** Light warm background (`#ffffff` / `#fcf8ed` cream / `#f8f2df` beige)
- **Accent:** Coral/orange `#cf6d47` — matches the Biorouter desktop app
- **Text:** Warm dark `#2a2520`, muted `#7a736c`
- **Fonts:** [Inter](https://fonts.google.com/specimen/Inter) (body) + [JetBrains Mono](https://fonts.google.com/specimen/JetBrains+Mono) (code) via Google Fonts
- **Responsive:** Mobile hamburger menu at ≤768px

---

## Hosting

> **This site now lives inside the main Biorouter app repo**, under `landing/`.
> It was consolidated from the standalone `BaranziniLab/biorouter-landing` repo so
> the website ships and versions together with the app and is easier for the AI
> agent to edit. It is published to **GitHub Pages via GitHub Actions**, served on
> the custom domain `biorouter.ucsf.edu`.

- **Repo:** <https://github.com/BaranziniLab/biorouter> (this folder: `landing/`)
- **Live URL:** <https://biorouter.ucsf.edu/>
- **Custom domain:** configured via the `CNAME` file in this folder — do **not** delete it; it tells GitHub Pages which domain owns the site. The deploy workflow uploads it with every build so the domain is re-asserted on each deploy.
- **Deploy workflow:** [`.github/workflows/deploy-landing.yml`](../.github/workflows/deploy-landing.yml) — uploads this `landing/` folder as the Pages artifact (served as-is, no Jekyll) and deploys it.
- **Build:** None required — fully static HTML/CSS/JS, no build step.

### One-time Pages setup (already done for the cutover, kept here for reference)

1. Release `biorouter.ucsf.edu` from the old `biorouter-landing` repo: its **Settings → Pages → remove the custom domain** (a domain can be attached to only one Pages site at a time).
2. In this repo: **Settings → Pages → Source = "GitHub Actions"**.

After that, the workflow runs on its own.

### Deploying updates

Just commit a change under `landing/` to `main` and push the **app** repo:

```bash
git add landing/
git commit -m "site: your message"
git push origin main
```

The `deploy-landing.yml` workflow triggers on any push to `main` that touches
`landing/**`, rebuilds Pages, and the change goes live within ~1–2 minutes. You
can also trigger it manually from the Actions tab (`workflow_dispatch`).

### Local preview

Open any HTML file directly in a browser, or use a local server to avoid cross-origin issues with relative paths:

```bash
# Python
python3 -m http.server 8080

# Node (npx)
npx serve .
```

Then visit `http://localhost:8080`.

---

## Acknowledgements

### The Baranzini Lab — [baranzinilab.ucsf.edu](https://baranzinilab.ucsf.edu/)

- Gianmarco Bellucci
- Sergio Baranzini

### Bakar Computational Health Sciences Institute (BCHSI) — [bakarinstitute.ucsf.edu](https://bakarinstitute.ucsf.edu/)

- Sharat Israni
- Marina Sirota

### UCSF Academic Research Services (ARS) — [ars.ucsf.edu](https://ars.ucsf.edu/)

- William Santo
- Evan Philps
- Rick Larson
- Oksana Gologorskaya

### Inspirations

Biorouter is an independent project that started from its own source code. The agents below influenced its design, Goose most of all, and it also draws on many open source libraries.

- **[Goose](https://block.github.io/goose/)**: CLI and desktop agent for full developer workflows (Block), and a major inspiration for Biorouter's design
- **[Claude Code](https://github.com/anthropics/claude-code)**: Anthropic's agentic coding tool for the terminal and IDE
- **[Codex CLI](https://github.com/openai/codex)**: OpenAI's open source coding agent that runs in the terminal
- **[Gemini CLI](https://github.com/google-gemini/gemini-cli)**: Google's open source AI agent for the terminal
- **[OpenHands](https://github.com/OpenHands/OpenHands)**: Open source platform for autonomous software development agents
- **[Aider](https://aider.chat/)**: Open source coding agent for the terminal that works with Git
- **[Cline](https://github.com/cline/cline)**: Open source interactive CLI coding agent
- **[OpenCode](https://opencode.ai/)**: Open source coding agent that supports multiple sessions and providers
- **[ForgeCode](https://forgecode.dev/)**: Terminal AI coding assistant for task planning and code generation

---

## Related

- **Biorouter app repo:** <https://github.com/BaranziniLab/biorouter>
- **Baranzini Lab:** <https://baranzinilab.ucsf.edu/>
- **UCSF Versa:** <https://ai.ucsf.edu/platforms-tools-and-resources/ucsf-versa>
