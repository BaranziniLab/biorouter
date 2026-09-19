# Built-in capabilities

This folder holds one user guide per capability that ships inside BioRouter. Each page explains how to confirm or change whether the capability is enabled, which tools it gives the agent, and a worked example. Most are enabled by default. Capabilities are part of BioRouter; they are not installed extensions. Third-party or user-installed connectors are extensions.

Come here when you want to know what a specific built-in capability can do, or why BioRouter reached for a tool you did not ask for. Go elsewhere if you are adding an extension BioRouter does not ship — installing a third-party MCP server, or writing a skill — which is covered by the [extensions, skills, and MCP agents guide](../extensions-and-skills-guide.md) one level up. If your question is about how much the agent is allowed to do rather than what it can do, the answer is in [security](../../security/README.md), not here.

## Documents

| Document | What it covers |
|----------|----------------|
| [Auto Visualiser](auto-visualiser.md) | How to enable the Auto Visualiser and which figures it produces, with a worked cohort-data example. The capability declares three tools — `render_figure`, `describe_figure` and `render_dashboard` — and reaches its 32 figure kinds through `render_figure`'s `kind`. |
| [Chat Recall](chat-recall.md) | Searching your past session history by keyword or session ID so BioRouter can pull earlier context into the current conversation. Unlike most built-ins, this one ships disabled by default. |
| [Code Execution](code-execution.md) | Code Mode: instead of calling MCP tools one at a time, the model writes a short JavaScript program that batches many tool calls into a single execution. |
| [Computer Use](computer-controller.md) | Native application observation, capture and control with per-chat task approval. |
| [Web & Documents](web-documents.md) | URL fetching, document utilities, and cache tools independent of the desktop runtime. |
| [Developer](developer.md) | A walkthrough of the Developer capability and its code, file, image, and shell tools, plus a reference on constraining it with permission modes, tool permissions, and `.biorouterignore`. |
| [Extension Manager](extension-manager.md) | How BioRouter discovers other extensions and attaches or detaches them mid-session so the active tool count stays small — and how it searches the trusted BAAM marketplace to install a package you do not have, or permanently delete one you no longer want. |
| [Memory](memory.md) | The trigger words that store, recall, and forget memories, where memories live on disk, and a worked example teaching BioRouter a lab's analysis standards. Predates the Knowledge feature; the page explains how the two relate. |
| [Skills](skills.md) | Where skills are discovered from on disk, how to get more of them, and a worked GWAS-pipeline example showing a skill steering the agent. |
| [Todo](todo.md) | How BioRouter breaks multi-step work into a tracked checklist and reports progress as it goes. |
| [Workspace Control](workspace.md) | The tools BioRouter uses to operate the workspace itself — list, open, read, steer and reconfigure other conversations, and delegate to subagents you can watch in a live tab. Ships in two tiers: delegation is automatic, the cross-session tools are an explicit opt-in. |

## Related documentation

- [Extensions, skills, and MCP agents](../extensions-and-skills-guide.md) — the guide to the three ways BioRouter is extended, and where to go to add or author an extension rather than use a bundled one.
- [Security](../../security/README.md) — permission modes, `.biorouterignore`, and secret storage; read it before letting the Developer or Computer Use capabilities run unattended.
- [Permission modes](../../security/permission-modes.md) — the specific mechanism for deciding whether BioRouter asks before acting, referenced from several pages in this folder.
- [Installation](../../getting-started/installation.md) — carries the authoritative list of which capabilities are enabled by default on a fresh install.
