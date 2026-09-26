# Crew user manual

> **What this is.** The index of the Crew user manual. It says what Crew is, where to start, and what each page covers.
> **Status:** Current.
> **Audience:** Lab members who use the Biorouter desktop app, the lab member who hosts a workspace, and the IT staff who support them.

Crew is the part of the Biorouter desktop app where a lab works together. You open it from the **Crew** item in the app sidebar. In Crew you chat in channels, share files, and ask your AI agent to do tasks that everyone in the channel can see. A lab's shared space is called a workspace. One lab member, the host, runs the workspace on a shared Linux server. Every other member connects to it with their own account on that server, over SSH (the secure login that lab servers use). Privacy in Crew is Private or Public. It controls which AI models may read the workspace. It does not control which people can see it: only people the host lets in can see a workspace.

## Start here

1. Read [Getting started](getting-started.md). It lists what you need before you begin and shows how the Crew screen is laid out.
2. Then read the page for your role:
   - If someone sent you an invitation, read [Joining a workspace](joining-a-workspace.md). You open Crew, choose **Join a workspace**, and paste the whole message your host sent you. Crew then shows you a code. You send that code to your host, and your host lets you in.
   - If you are setting up Crew for your lab, read [Hosting a workspace](hosting-a-workspace.md). You open Crew and choose **Host a new workspace**. Then you invite each person by their username on the server, send them the invitation message Crew writes, and choose **Let in…** when they send you their code.
   - If you support Crew on a server, read [Administration](administration.md). For the `biorouter crew` terminal commands, read [Command line](command-line.md).

## Pages in this manual

| Page | What it covers |
|---|---|
| [Getting started](getting-started.md) | What you need before you use Crew, the first screen, and the parts of the Crew window. |
| [Hosting a workspace](hosting-a-workspace.md) | Creating a workspace for your lab, inviting people, letting them in with their code, and the setup checklist. |
| [Joining a workspace](joining-a-workspace.md) | Pasting your host's invitation, sending your code, and what happens while you wait to be let in. |
| [Teams, channels and people](teams-channels-and-people.md) | Creating teams and channels, adding and removing people, channel owners, archiving, and your display name. |
| [Messages and files](messages-and-files.md) | Writing and reading messages, unread counts, drafts, and sharing files up to 1 GB. |
| [Agents and chat access](agents-and-chat-access.md) | Asking your agent to do a task in a channel, connecting an ordinary Biorouter chat with `/crew`, and ending that access. |
| [Privacy and security](privacy-and-security.md) | Private and Public, institutions, "Restricted" and "Public-safe" channels, keys and fingerprints, and what the host can see. |
| [Connections and troubleshooting](connections-and-troubleshooting.md) | What each connection status means, signing in, checking the server's identity, and what to do about common error messages. |
| [Command line](command-line.md) | The `biorouter crew` commands, for people who prefer a terminal or want to script Crew. |
| [Administration](administration.md) | Server requirements, installing `biorouter-crew` in each account, where Crew keeps its data, limits, upgrades and backups. |

## Related documentation

- [Installation and setup](../getting-started/installation.md): install the Biorouter desktop app, which includes Crew.
- [Privacy tiers](../security/privacy-tiers.md): the rules Biorouter uses across the whole app to keep private data away from public models. Written for developers.
- [BioRouter Crew implementation and evidence](../research/biorouter-crew/README.md): the design, the implementation plan and the test evidence behind Crew. Written for developers.
