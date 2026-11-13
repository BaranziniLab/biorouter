# Usage tips

> **What this is.** A collection of short, independent tips for working with biorouter day to day — prompting, model choice, context and chat hygiene, extensions, safety, cost, and workflows.
> **Status:** Current.
> **Audience:** end users

Each tip below stands alone; read the group that matches what you are doing rather than working through the page in order. The tips assume you already have biorouter installed and a provider configured — if not, start with [biorouter in 5 minutes](quickstart.md).

MCP (Model Context Protocol) is the open standard biorouter uses to connect to pluggable tool servers, which it calls *extensions*.

## Working with biorouter

### biorouter works on your behalf

biorouter is an AI agent, which means you can prompt biorouter to perform tasks for you like opening applications, running shell commands, automating workflows, writing code, browsing the web, and more.

### Prompt biorouter using natural language

You don't need fancy language or special syntax to prompt biorouter. Talk with biorouter like you would talk to a friend. You can even use slang or say please and thank you; biorouter will understand.

### Embrace an experimental mindset

You don't need to get it right the first time. Iterating on prompts and tools is part of the workflow.

## Choosing and managing models

### Choose the right LLM

Your experience with biorouter is shaped by your choice of LLM, as it handles all the planning while biorouter manages the execution. When choosing an LLM, consider its tool support, specific capabilities, and associated costs. See [Choosing a model provider](choosing-a-model-provider.md).

### Pair two models to save money

Choose a [default worker model](../agent-loop/subagents.md#model-control) in Settings > Models. Your chat keeps its own model for planning and conversation, and the subagents it starts run on the lighter, lower-cost model you chose unless the agent names another.

## Managing context and chats

### Keep chats short

LLMs have context windows, which limit how much chat history they can retain. Once exceeded, they may forget earlier parts of the chat. Monitor your token usage and [start new chats](managing-sessions.md) as needed.

### Use Quick Launcher for faster chat starts

Press `Cmd+Option+Shift+G` (macOS) or `Ctrl+Alt+Shift+G` (Windows/Linux) and send a prompt to start a new chat instantly.

### Teach biorouter your preferences

Help biorouter remember how you like to work by using [`.biorouterhints` and other context files](../agent-loop/context-engineering.md) or [skills](../extensions/built-in/skills.md) for permanent project preferences, and the [Memory capability](../extensions/built-in/memory.md) for things you want biorouter to dynamically recall later. Both can help save valuable context window space while keeping your preferences available.

### Turn off unnecessary extensions and tools

Turning on too many extensions can degrade performance. Enable only essential [extensions and tools](../extensions/extensions-and-skills-guide.md) to improve tool selection accuracy, save context window space, and stay within provider tool limits.

> **Tip.** Consider enabling [Code Mode](../extensions/built-in/code-execution.md), an alternative approach to tool calling that discovers tools on demand.

## Extending biorouter

### Extend biorouter's capabilities to any application

biorouter's capabilities are extensible. As an [MCP](https://modelcontextprotocol.io/) client, biorouter can connect to your apps and services through [extensions](../extensions/extensions-and-skills-guide.md), allowing it to work across your entire workflow.

## Building reusable workflows

### Set up starter templates

You can turn a successful chat into a reusable "[workflow](../workflows/creating-and-sharing-workflows.md)" to share with others or use again later — no need to start from scratch.

### Make workflows safe to re-run

Write [workflows](../workflows/creating-and-sharing-workflows.md) that check your current state before acting, so they can be run multiple times without causing any errors or duplication.

### Add logging to workflows

Include informative log messages in your workflows for each major step to make debugging and troubleshooting easier should something fail.

## Staying safe

### Choose how much control biorouter has

You can customize how much [supervision](../security/permission-modes.md) biorouter needs. Choose between full autonomy, requiring approval before actions, or simply chatting without any actions.

### Protect sensitive files

biorouter is often eager to make changes. You can stop it from changing specific files by creating a `.biorouterignore` file, listing all the file paths you want it to avoid. See the [Developer capability](../extensions/built-in/developer.md) for how these access controls are applied.

### Control which extensions biorouter can use

Administrators can use an allowlist to restrict biorouter to approved extensions only. This helps prevent risky installs from unknown MCP servers. See [Managed / enterprise policy](../security/managed-policy.md) for the centrally administered allow, ask, and deny rules.

### Commit changes early and often

Commit your code changes early and often. This allows you to rollback any unexpected changes.

## Keeping your install current

### Keep biorouter updated

Regularly update biorouter to benefit from the latest features, bug fixes, and performance improvements. See [Update Biorouter](installation.md#update-biorouter) for the desktop and CLI upgrade paths.

## Related documentation

- [biorouter in 5 minutes](quickstart.md) — the first-task walkthrough these tips assume you have completed.
- [Choosing a model provider](choosing-a-model-provider.md) — the provider and model details behind the "choose the right LLM" tip.
- [Context engineering](../agent-loop/context-engineering.md) — the full treatment of hints, skills, memory, and workflows touched on above.
- [Permission modes](../security/permission-modes.md) — how to set the level of supervision biorouter needs.
- [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md) — what to do when a tip above does not resolve the behaviour you are seeing.
