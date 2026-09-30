# Getting started with Crew

> **What this is.** The first page of the Crew user manual: what you need, the background service, the first screen, the parts of the Crew view, and the terms the manual uses.
> **Status:** Current. Checked against the Crew code on 2026-09-29.
> **Audience:** Lab members who are new to Crew, including people who have never used SSH or a terminal.

Crew is the part of the Biorouter desktop app where your lab chats, shares files and runs AI agents. Open it from **Crew** in the Biorouter sidebar, below **New chat**. As in Slack, a workspace holds channels where you post messages and files. Teams group channels, and agents read and post in a channel when you allow it. You see only the channels you are in. To get into another one, ask its owner. The host can also add you, but only to a team or channel the host is in.

One lab member, the host, runs the workspace on a shared Linux server. You connect with your own account on that server over SSH (Secure Shell, the standard way to sign in to a server), and Crew makes that connection for you. The desktop app and the `biorouter crew` [commands](command-line.md) share one background service, so they show the same workspaces.

Crew works only in the desktop app and with `biorouter crew`. It does not work in a web browser opened with `biorouter serve`. There, **Crew** in the sidebar opens a page titled "Crew needs the Biorouter desktop app" in place of your workspaces. Every Crew action needs proof that a person is at this computer, which the desktop app and `biorouter crew` supply themselves and the background service that `biorouter serve` starts never holds, so nothing you do in the browser changes that, and signing in again or restarting does not help. Open Crew in the desktop app on your own computer, or use `biorouter crew` in a terminal there.

A word in braces is a name Crew fills in: "Connect to {workspace}" appears as "Connect to lab".

## What you need

| You need | From |
|---|---|
| The Biorouter desktop app ([Installation and setup](../getting-started/installation.md)) | You or IT |
| An account on the lab's Linux server, with its SSH key, password or verification code | Your IT team |
| The server's identity in your [known hosts file](#glossary-of-crew-terms) ([Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer)). Crew never accepts a new server by itself. | You, with the fingerprint from IT |
| `biorouter-crew` in your own account on the server ([Install Crew in your server account](joining-a-workspace.md#install-crew-in-your-server-account)) | You or IT |
| The host's invitation message, valid for 24 hours | The host |
| For a Private workspace, your institution's short ID, such as `ucsf`. The invitation usually fills it in. | The host |

### Signing in to the server

When the server asks for a password or a code, the Sign in window opens. Type it there. Nothing you type is saved. For help, choose **Trouble signing in?** there, or see [Sign in to the server](connections-and-troubleshooting.md#sign-in-to-the-server). In a terminal, `biorouter crew auth` signs you in instead ([Sign in and stay connected](command-line.md#sign-in-and-stay-connected)).

### Jump hosts

Use a jump host, a server you pass through to reach the lab server, only if IT gives you one. Type it under **Advanced** in the Join or Host dialog, in **Jump hosts**, with commas between several, or later in **Connection settings…** in the workspace menu. IT must also add its settings to `~/.ssh/config` (see [SSH requirements](administration.md#ssh-requirements)). Without them, or with a custom `ProxyCommand` or `GSSAPIDelegateCredentials yes`, Crew does not connect and shows a sentence that starts "Crew SSH host" and names the jump host. Send that sentence to IT. Those settings also make SSH check the jump host strictly, so verify it on its own before your first connection (step 3 of [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer)).

## The background service

Biorouter's background service (the daemon) keeps your workspaces connected. On a Mac or Linux computer it starts by itself when you first open Biorouter, and it keeps running after you quit, so Crew stays connected. The desktop app and `biorouter crew` connect to it without asking you anything.

- After your computer restarts, the service starts again when you open Biorouter or run a `biorouter crew` command.
- After you update Biorouter, the new version replaces the older service the next time you open Biorouter. Replacing it may interrupt a running agent task or file transfer, so check [your tasks](agents-and-chat-access.md#follow-a-task) and [your transfers](messages-and-files.md#transfer-states) afterward.
- On Windows, quitting Biorouter stops the service.

If a "Biorouter Error" window appears when you open Biorouter, its message names the cause, and Biorouter closes when you close it.

## The first screen

The first time, Crew shows "Work together in Crew" with a **Join a workspace** button and a **Host a new workspace** link. Most people join.

- To join, follow [Joining a workspace](joining-a-workspace.md).
- To host, follow [Hosting a workspace](hosting-a-workspace.md).

After that, the main area shows what to do next. [What each status means](connections-and-troubleshooting.md#what-each-status-means) explains each connection status, and [When joining does not work](joining-a-workspace.md#when-joining-does-not-work) covers the screens you may see while you join.

## A tour of the Crew view

| Part | What it does |
|---|---|
| Workspace name | Opens the workspace menu: **People…**, **Privacy…** and **Agent access…** (tabs of the "{workspace} settings" dialog), **Create team…**, the host's **Invite people to {workspace}…**, the [connection tools](connections-and-troubleshooting.md#connection-tools-in-the-workspace-menu), **Switch workspace** and **Add a workspace**. |
| Status row | One word for your connection, such as "Connected" or "Offline" ([What each status means](connections-and-troubleshooting.md#what-each-status-means)). **Sign-in needed** is a button. The [privacy chip](#the-privacy-chip) sits at its right. |
| "Invitations", "Waiting to join", "Joined, not in your teams" | Invitations for you (**Join**). For the host, people to let in (**Let in…**) or add to a team (**Add to a team…**). |
| Teams and channels | Bold names with a count have unread messages. A pencil marks unsent text, and the count takes its place while the channel has unread messages ([Find your way around the sidebar](teams-channels-and-people.md#find-your-way-around-the-sidebar)). |
| Agents | Your unfinished tasks and chats with access. "Needs you" marks a task that waits for your approval ([See which agents have access](agents-and-chat-access.md#see-which-agents-have-access)). |
| You row | **Edit profile…**, **Copy my username** and **Keys and security…**, which lists your devices and where your keys are stored. Only a new Crew profile can use an [encrypted vault](privacy-and-security.md#use-an-encrypted-vault) instead of the system keychain. |
| Channel header | The channel name opens the channel menu. A "Restricted" or "Public-safe" badge follows. **Channel details** opens the **About**, **Members**, **Files** and **Agent access** tabs. |
| Connection bar | Under the channel header. It shows a problem with **Retry**, **Try again** or **Dismiss**, a locked vault with **Unlock**, or a new device with **Review** ([Messages and what to do](connections-and-troubleshooting.md#messages-and-what-to-do)). |
| Message box | Enter sends. Shift+Enter adds a line. **Attach** adds a file of up to 1 GB or a server path, shared when you press Send. **Ask my agent** starts a task everyone in the channel sees ([Messages and files](messages-and-files.md)). |

Outside Crew, the **Crew** item in the app sidebar shows how many unread messages you have, and new messages raise a system notification ([Unread messages](messages-and-files.md#unread-messages)). A chat with access shows a **Crew · #{channel}** chip. Choose it to manage that access.

### The privacy chip

The chip reads "Private · {institution}", such as "Private · UCSF", or "Public". Privacy decides which AI models may read the workspace, not which people see it. Public applies only when both your connection and the workspace allow it. The chip reads "Checking privacy…" while Crew checks, and is hidden while you are offline or waiting to join. Choose it for details ([Privacy and security](privacy-and-security.md#check-a-workspaces-privacy)).

### Where Crew keeps your place

- Crew opens on the first workspace under **Switch workspace** in the workspace menu.
- Each workspace reopens on the channel you last had open.
- Unsent text stays in each message box until you quit Biorouter.

If support staff ask for an ID, **Copy for support** in the menu of that person, team, channel or message copies it. For keyboard use, see [the sidebar](teams-channels-and-people.md#use-the-keyboard-in-the-sidebar) and [the message list](messages-and-files.md#use-the-keyboard-in-the-message-list).

## Glossary of Crew terms

| Term | Meaning |
|---|---|
| Agent | An AI model working for you. It reads and posts in a channel only when you allow it. |
| Background service (daemon) | The Biorouter program that keeps running after you quit the app. The app and `biorouter crew` share it and connect to it by themselves. See [The background service](#the-background-service). |
| Channel owner | The person who adds and removes a channel's members and can rename or archive it. At first, its creator. |
| Chat access | Lets an ordinary chat read and post in a channel after you type `/crew` in it. It ends when you revoke it, or after an hour. |
| Connection | This computer's saved link to one workspace: your server login, SSH settings, privacy choice and institution. Change it in **Connection settings…**. |
| Crew profile | Your Crew keys and saved workspaces on this computer. A new one has no keys yet. |
| Device | A computer where you use Crew, each with its own key. **Keys and security…** lists yours. |
| Fingerprint | A value you compare with a trusted copy to confirm an identity. Workspace and server fingerprints differ. Neither is your code. |
| Host | The lab member who runs the workspace in their server account. Only the host invites people and lets them in. |
| Institution | Your organization's short ID, such as `ucsf`. A Private workspace needs it before agents can work there. |
| Invitation | The host's message that lets you join. It is valid for 24 hours. |
| Jump host | A server you pass through to reach the lab server. See [Jump hosts](#jump-hosts). |
| Known hosts file | The list of servers SSH trusts on this computer, usually `~/.ssh/known_hosts`. Crew connects only to a server listed there. |
| Online | A person whose computer is connected to the workspace now, or made a request in the last three minutes. A dot after the name marks them in member lists. |
| Restricted, Public-safe | Channel labels. Only private models read "Restricted". Public models may read "Public-safe" when the workspace allows. Neither limits who is in the channel. |
| System keychain | Your computer's own password store, such as the macOS Keychain. Crew keeps your keys there unless you set up a vault. |
| Task | One job you give your agent with **Ask my agent**. Everyone in the channel sees it. |
| Team | A group of channels in a workspace. Anyone can create one. |
| Vault | An encrypted file on this computer, with its own passphrase, that holds your Crew keys in place of the system keychain. |
| Workspace | Your lab's shared space on the server, with its people, teams and channels. |
| Your code | 16 letters and numbers (`XXXX-XXXX-XXXX-XXXX`) that the host enters with **Let in…**. It stays the same when you reconnect or close the app, and never contains I, L, O or U. |

## Related documentation

- [Crew user manual](README.md): every page in the manual.
- [Joining a workspace](joining-a-workspace.md): join with an invitation.
- [Hosting a workspace](hosting-a-workspace.md): create a workspace and let people in.
- [Teams, channels and people](teams-channels-and-people.md): manage teams and members.
- [Messages and files](messages-and-files.md): messages and file sharing.
- [Agents and chat access](agents-and-chat-access.md): agent tasks and connected chats.
- [Privacy and security](privacy-and-security.md): Private and Public, keys, fingerprints.
- [Connections and troubleshooting](connections-and-troubleshooting.md): every status and error.
- [Command line](command-line.md): the `biorouter crew` commands.
- [Administration](administration.md): server setup and installation.
