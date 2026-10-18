# Crew user manual

> **What this is.** The Crew manual's index: what Crew is, where to start, and every page.
> **Status:** Current.
> **Audience:** Lab members, workspace hosts, and their IT staff.

Crew is where your lab works together in the Biorouter desktop app. Open **Crew** in the app sidebar. You chat in channels, share files, and give your AI agent tasks the whole channel can see. The `biorouter crew` [commands](command-line.md) do the same from a terminal. Crew does not work in a web browser opened with `biorouter serve`: it shows "Crew needs the Biorouter desktop app" there, for the reason [Getting started](getting-started.md) gives.

A lab's shared space is a workspace. The host, a lab member, runs it on a Linux server. Each member connects with their own server account over SSH (the secure login lab servers use). Only people who were let in can see it: by the host, or in an open workspace by any member.

Privacy, **Private** or **Public**, controls which AI models may read the workspace, not which people see it. Your connection and the workspace each have a setting, and the stricter one applies.

## Start here

1. [Install the Biorouter desktop app](../getting-started/installation.md).
2. Read [Getting started](getting-started.md).
3. Follow your role's steps below. IT staff start with [Administration](administration.md).

You do not install anything on the server yourself. When you join or host, Biorouter installs Crew's server program in your own server account, with no administrator rights, and asks for your server password in its Sign in window if the server wants one.

### Join a workspace

1. Send your host your exact username on the lab server.
2. [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer).
3. Open Crew and choose **Join a workspace**.
4. Paste the whole invitation message your host sends you.
5. Check **Your agent on {server}**, which says what your agent may do on the server ([Agent access levels](joining-a-workspace.md#your-agent-on-the-server)).
6. Choose the Join button, for example **Join chen-lab**. If Crew is not in your server account yet, Biorouter installs it there. Crew then shows a code.
7. Send the code to your host.

When you are let in, the workspace opens. In an open workspace, the kind a new workspace is, you are in every team and channel at once. In a managed one, your host adds you to a team. See [Joining a workspace](joining-a-workspace.md).

### Host a workspace

1. [Verify the server](joining-a-workspace.md#verify-the-server-on-this-computer) on this computer.
2. Open Crew, choose **Host a new workspace**, and follow the dialog. In its second step, **Set up and start Crew** installs Crew in your server account and starts the workspace. In a terminal, one command does the whole job: `biorouter crew host --name chen-lab --server alice@hpc.example.edu --institution ucsf` ([Host a workspace](command-line.md#host-a-workspace)).
3. Invite each person by server username. Send them Crew's invitation message and the server's `SHA256:` fingerprint from IT.
4. When a person sends their code, choose **Let in…** beside their name and enter it.

The new workspace is open: every member can manage teams and channels and let people in, and a person who joins is added to every team and channel. You can make it managed instead. See [Hosting a workspace](hosting-a-workspace.md).

## Pages in this manual

| Page | What it covers |
|---|---|
| [Getting started](getting-started.md) | What you need, the first screen, the Crew view, and a glossary. |
| [Hosting a workspace](hosting-a-workspace.md) | Setting up and starting Crew, creating a workspace, open and managed workspaces, inviting and letting people in, keeping Crew running, and deleting the workspace. |
| [Joining a workspace](joining-a-workspace.md) | The invitation, what your agent may do on the server, your code, waiting to be let in, and fixing a failed join. |
| [Teams, channels and people](teams-channels-and-people.md) | Who may do what in an open or managed workspace, teams, channels, browsing and joining channels, adding and removing people, who is online, archiving, deleting, and display names. |
| [Messages and files](messages-and-files.md) | Messages, mentions, unread counts and notifications, drafts, files up to 1 GB, a channel's files, server paths, and what happens to a deleted channel. |
| [Agents and chat access](agents-and-chat-access.md) | Asking your agent for tasks with **Ask my agent** or `@agent`, the model for Crew tasks, what your agent may do on the server, the workspace context, connecting a Biorouter chat with `/crew`, and ending access. |
| [Privacy and security](privacy-and-security.md) | **Private** and **Public**, institutions, **Restricted** and **Public-safe** channels, keys, fingerprints, what others, including the host and other accounts on the server, can see, and what full agent access and open workspaces risk. |
| [Connections and troubleshooting](connections-and-troubleshooting.md) | Connection statuses, signing in, checking the server's identity, setting Crew up in your server account, a deleted workspace, and error messages. |
| [Command line](command-line.md) | The `biorouter crew` commands for terminals and scripts. |
| [Administration](administration.md) | Server requirements, how Biorouter installs `biorouter-crew`, storage permissions, what other accounts can see, a full disk, where Crew keeps data, limits, upgrades, deletion and backups. |

## Related documentation

- [Privacy tiers](../security/privacy-tiers.md): the app's privacy rules, for developers.
- [Crew design and reference](design/README.md): the broker protocol, naming design, interface specification and CLI guide, for developers.
- [Crew build campaign](../history/biorouter-crew/README.md): the plans, reviews and test evidence of the campaign that built Crew.
