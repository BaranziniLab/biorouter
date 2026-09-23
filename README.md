<div align="center">

<img src="ui/desktop/src/images/icon.png" alt="Biorouter" width="120"/>

# UCSF Biorouter

**An open-source AI workspace for biomedical research**

<p>
  <a href="https://opensource.org/licenses/Apache-2.0"><img src="https://img.shields.io/badge/License-Apache_2.0-blue.svg" alt="Apache 2.0 License"></a>
  <img src="https://img.shields.io/badge/version-1.91.1-tan.svg" alt="Version 1.91.1">
</p>

<a href="https://biorouter.ucsf.edu/download">Download</a> ·
<a href="https://biorouter.ucsf.edu/docs">Documentation</a> ·
<a href="https://biorouter.ucsf.edu/baam">BAAM Marketplace</a> ·
<a href="https://biorouter.ucsf.edu/">Website</a>

</div>

Biorouter brings AI models, research tools, and your data into one workspace. Built at the [Baranzini Lab](https://baranzinilab.ucsf.edu/) at UCSF, it runs as a desktop app, a terminal CLI, or a browser interface.

## Choose your model

Use the models you prefer, with the same workspace and research tools.

| Option | Examples |
|---|---|
| **Private institutional models** | UCSF Versa API Azure and Versa API Bedrock |
| **Private local models** | Bundled Llama Server or Ollama, running on your computer |
| **Commercial APIs** | OpenAI, Anthropic, Google, Azure OpenAI, Amazon Bedrock, OpenRouter, and more |
| **Coding-agent subscriptions** | Claude Code and Codex through their installed, signed-in command-line tools |

You can also add compatible custom endpoints. Local models are private when they run on your machine; institutional models are private when they use the approved UCSF gateway. Claude Code and Codex use cloud inference and are treated as public providers.

[Model setup](docs/desktop-ui/provider-catalog.md) · [Claude Code and Codex setup](docs/providers/coding-agents/installing-and-signing-in.md)

## Keep private work private

Privacy protection is on by default and applies across chats, knowledge bases, extensions, and subagents.

- **Private chats stay private.** Once a chat uses a private model or data source, Biorouter prevents it from switching to a public model. Changing its classification requires a deliberate user action.
- **Public models cannot retrieve private context.** Biorouter blocks access to private chats and knowledge bases, and refuses extensions marked private.
- **Delegation respects the boundary.** An agent cannot use a subagent to move work between private and public models.

These controls help prevent accidental disclosure within Biorouter. They do not isolate ordinary files or the physical desktop: a model with shell or approved desktop access can encounter sensitive material there. For patient data or unpublished research, use an appropriate local or institutional model and follow your institution's requirements.

[Privacy protections and limits](docs/security/privacy-tiers.md#what-shipped-and-what-did-not) · [Tool permissions](docs/security/permission-modes.md)

## Built for biomedical research

The [BAAM marketplace](https://biorouter.ucsf.edu/baam) offers AI agents, extensions, and skills for literature review, genomics, single-cell analysis, statistics, and clinical research. Connect tools through the Model Context Protocol (MCP), or add your lab's own workflows and instructions.

Work with papers, genes, diseases, drugs, cohorts, and experimental data in the same conversation. Connectors such as SPOKEAgent, UCSF OMOP Agent, and CDWAgent bring knowledge graphs and institutional electronic health records into your research, subject to the access each service requires.

[Browse the marketplace](https://biorouter.ucsf.edu/baam) · [Extensions and skills](docs/extensions/extensions-and-skills-guide.md)

## What you can do

| Task | In Biorouter |
|---|---|
| **Build a research knowledge base** | Ingest papers and documents into linked Markdown pages with source references and version history. Choose BioOKF, the Biomedical Open Knowledge Format, for structured biomedical knowledge. |
| **Explore clinical data** | Query authorized UCSF OMOP and Clinical Data Warehouse sources and build cohorts through their agents. |
| **Connect biological evidence** | Use SPOKEAgent to explore relationships among genes, diseases, drugs, and pathways. |
| **Analyze and visualize** | Write and run R or Python, fit machine-learning models, and create figures and interactive dashboards. |
| **Work with desktop software** | Biorouter Copilot can inspect and operate applications on the backend computer, with your approval for each request. |
| **Build and reuse tools** | Create small research apps with Agent Drafter, run parallel chats, and save or schedule recurring tasks as workflows. |

Copilot keeps observations and approvals separate for each chat. The applications themselves share a desktop, so choose the model before opening sensitive records or research data. See [Copilot setup and limits](docs/extensions/built-in/computer-controller.md).

## Get started

1. **[Download Biorouter](https://biorouter.ucsf.edu/download)** for macOS, Windows, or Linux.
2. **Choose a model.** UCSF users can select **Versa API Azure** or **Versa API Bedrock** under Institutional Models. For local inference, choose **Llama Server** and download a model. You can also connect a commercial provider.
3. **Start a chat.** Add a paper or dataset, or install an agent from BAAM.

For example, attach a few papers and ask:

> Summarize these papers and compare the evidence for each proposed mechanism. Link each finding to its source.

The desktop app includes the CLI. After [adding it to your PATH](docs/cli/command-reference.md#setup-path), you can start from a terminal:

```bash
biorouter configure
biorouter
```

For browser access, run `biorouter serve`. It listens on localhost by default. See [browser and server setup](docs/deployment/browser-access.md) for remote access and CLI-only Linux packages.

## Learn more and contribute

Biorouter uses a shared Rust agent core, an Electron and React desktop app, and MCP extensions. The `biorouterd` backend serves the desktop and browser interfaces through REST and WebSocket APIs.

- [Documentation](docs/README.md): guides and references by topic.
- [Architecture](docs/architecture/system-overview.md): how the components fit together.
- [Knowledge bases and BioOKF](docs/knowledge-base/README.md), [workflows](docs/workflows/README.md), and [Apps SDK](docs/apps-sdk/sdk-reference.md).
- [Contributing](CONTRIBUTING.md): build from source, test, and submit changes.
- [Report an issue](https://github.com/BaranziniLab/biorouter/issues). Report security concerns privately using [SECURITY.md](SECURITY.md).

## Credits and citation

Developed by **Wanjun Gu** at UCSF's [Baranzini Lab](https://baranzinilab.ucsf.edu/), with support from UCSF IT and Information Commons.

Biorouter is a fork of Block's [Goose](https://github.com/block/goose), licensed under [Apache 2.0](LICENSE). It builds on open-source projects including [Open Computer Use](https://github.com/iFurySt/open-codex-computer-use), [llama.cpp](https://github.com/ggml-org/llama.cpp), and the visualization libraries listed in [NOTICE](NOTICE). See [acceptable use](ACCEPTABLE_USAGE.md) and [project governance](GOVERNANCE.md).

If you use Biorouter in your research, please cite:

```bibtex
@software{biorouter2025,
  title  = {UCSF Biorouter: An AI-Powered Integrated Research Environment},
  author = {Gu, Wanjun and Bellucci, Gianmarco and Baranzini, Sergio E.},
  year   = {2025},
  url    = {https://github.com/BaranziniLab/biorouter}
}
```
