# Extensions and skills

This folder documents BioRouter's three kinds of add-on context: built-in **capabilities**, installed **extensions**, and reusable **skills**. Capabilities ship with BioRouter; extensions are user-installed or third-party MCP connectors; skills teach procedures without adding tools. It holds the end-user guide to all three, a reference page for each built-in capability, and an open investigation into a Slack integration that has not been built.

Come here when you want to install an extension, understand a shipped capability, or author a skill. Agent Drafter is documented in [`docs/agent-drafter/`](../agent-drafter/README.md) and its typed app surface in [`docs/apps-sdk/`](../apps-sdk/README.md). If you are trying to restrict what any tool is allowed to do, go to [`docs/security/`](../security/README.md); for a scripted repeatable task, go to [`docs/workflows/`](../workflows/README.md).

## Documents in this folder

| Document | What it covers |
|---|---|
| [Capabilities, extensions, skills, and MCP agents](extensions-and-skills-guide.md) | The end-user guide to what ships, what is installed, and what supplies reusable instructions, including BAAM and remote MCP agents. |
| [Installing an extension, and where its credentials go](installing-an-extension.md) | How a `.brxt` install works from each of the four surfaces that can do it — the desktop marketplace, the local file drop, the CLI and an agent in chat — and the design of the credential path: why a secret never enters the conversation, what enforces that rather than documents it, and what it costs (a browser daemon cannot accept credentials over HTTP). |
| [The skill catalog](skill-catalog.md) | How BioRouter decides which skills exist and which are on: the five roots, the daemon-served catalog every surface reads, the difference between a machine-wide and a per-chat choice, and how a skill installed mid-conversation becomes usable in it. |
| [Skill packages](skill-packages.md) | Importing a skill, or a coordinated package of skills, from a repository URL, a ZIP, the marketplace, the CLI or an agent: the detection ladder, why ambiguity is a question rather than a default, and what lands on disk. |
| [Reliable Slack posting from the agent](slack-posting-investigation.md) | An options memo comparing three ways to give the agent reliable Slack posting — incoming webhook, a Slack MCP server with a bot token, and a user token or the Slack CLI — with a recommendation. **Open investigation, not implemented:** no `slack_post` tool or Slack extension exists in the tree, and the recommendation has been neither accepted nor rejected. |

## Built-in capability reference

The [`built-in/`](built-in/README.md) subdirectory holds one user-facing reference page per built-in capability: Developer, Biorouter Copilot, Web & Documents, Memory, Auto Visualiser, Chat Recall, Code Execution, Extension Manager, Skills, Todo and Workspace Control.

Two pages are worth knowing about before you go. [Developer](built-in/developer.md) carries the most substantive security guidance, and [Biorouter Copilot](built-in/computer-controller.md) documents the highest-blast-radius capability because it acts on your real desktop.

## Related documentation

- [Installation](../getting-started/installation.md) — the default-enabled capability list a new install starts from, before you add extensions
- [Permission modes](../security/permission-modes.md) — how to decide whether BioRouter asks before `shell`, `text_editor` or native desktop actions acts on your machine
- [Agent Drafter](../agent-drafter/README.md) — the built-in capability that builds interactive apps, documented separately because of its size
- [Integrations with other tools](../integrations/README.md) — the opposite direction: adapters that let another host application, such as JupyterLab's Jupyter AI chat, run a Biorouter agent with everything documented here already attached
- [Workflows](../workflows/README.md) — the scripted alternative to a skill when you want deterministic steps rather than agent judgement
