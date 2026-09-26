# Crew user manual

> **What this is.** The Crew manual's index: what Crew is, where to start, and every page.
> **Status:** Current.
> **Audience:** Lab members, workspace hosts, and their IT staff.

Crew is where your lab works together in the Biorouter desktop app. Open **Crew** in the app sidebar. You chat in channels, share files, and give your AI agent tasks the whole channel can see.

A lab's shared space is a workspace. The host, a lab member, runs it on a Linux server. Each member connects with their own server account over SSH (the secure login lab servers use). Only people the host lets in can see it.

Privacy, **Private** or **Public**, controls which AI models may read the workspace, not which people see it. Your connection and the workspace each have a setting, and the stricter one applies.

## Start here

1. [Install the Biorouter desktop app](../getting-started/installation.md).
2. Read [Getting started](getting-started.md).
3. Follow your role's steps below. IT staff start with [Administration](administration.md).

### Join a workspace

1. Send your host your exact username on the lab server.
2. [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer).
3. Open Crew and choose **Join a workspace**.
4. Paste the whole invitation message your host sends you.
5. Choose the Join button, for example **Join chen-lab**. Crew shows a code.
6. Send the code to your host.

When your host lets you in and adds you to a team, your team's channels appear. See [Joining a workspace](joining-a-workspace.md).

### Host a workspace

1. [Install Crew](hosting-a-workspace.md#install-crew-on-the-server) in your server account, and [verify the server](joining-a-workspace.md#verify-the-server-on-this-computer) on this computer.
2. Open Crew, choose **Host a new workspace**, and follow the dialog and the checklist after it.
3. Install Crew in each person's server account too, or they see "Crew isn’t set up for your account" and cannot join.
4. Invite each person by server username. Send them Crew's invitation message and the server's `SHA256:` fingerprint from IT.
5. When a person sends their code, choose **Let in…** beside their name and enter it.
6. When Crew says the person joined, tick channels and choose **Add to {team}**. A person in no team sees no channels.

Crew then says the person can now see the team's channels. See [Hosting a workspace](hosting-a-workspace.md).

## Pages in this manual

| Page | What it covers |
|---|---|
| [Getting started](getting-started.md) | What you need, the first screen, the Crew view, and a glossary. |
| [Hosting a workspace](hosting-a-workspace.md) | Creating a workspace, inviting and letting people in, and keeping Crew running. |
| [Joining a workspace](joining-a-workspace.md) | The invitation, your code, waiting for your host, and fixing a failed join. |
| [Teams, channels and people](teams-channels-and-people.md) | Teams, channels, adding and removing people, channel owners, archiving, and display names. |
| [Messages and files](messages-and-files.md) | Messages, unread counts, drafts, files up to 1 GB, and server paths. |
| [Agents and chat access](agents-and-chat-access.md) | Asking your agent for tasks, connecting a Biorouter chat with `/crew`, and ending access. |
| [Privacy and security](privacy-and-security.md) | **Private** and **Public**, institutions, **Restricted** and **Public-safe** channels, keys, fingerprints, and what others, including the host, can see. |
| [Connections and troubleshooting](connections-and-troubleshooting.md) | Connection statuses, signing in, checking the server's identity, and error messages. |
| [Command line](command-line.md) | The `biorouter crew` commands for terminals and scripts. |
| [Administration](administration.md) | Server requirements, installing `biorouter-crew`, where Crew keeps data, limits, upgrades and backups. |

## Related documentation

- [Privacy tiers](../security/privacy-tiers.md): the app's privacy rules, for developers.
- [BioRouter Crew implementation and evidence](../research/biorouter-crew/README.md): Crew's design, plan and test evidence, for developers.
