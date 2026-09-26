# Getting started with Crew

> **What this is.** The first page of the Crew user manual. It lists what you need before you start, explains the first screen, walks through each part of the Crew view, and defines the words the rest of the manual uses.
> **Status:** Current. Checked against the Crew code on 2026-09-25.
> **Audience:** Lab members who are new to Crew, including people who have never used SSH or a terminal.

Crew is the part of the Biorouter desktop app where your lab chats, shares files and runs AI agents together. One lab member, the host, runs the workspace on a shared Linux server. Everyone else reaches it through their own account on that server. Crew makes that connection for you, so your daily work happens in the app and not in a terminal.

In this manual, a word in braces stands for a name that Crew fills in. For example, "Connect to {workspace}" appears on your screen as "Connect to lab" when your workspace is called `lab`.

## What Crew is

- You open Crew from the **Crew** item in the Biorouter sidebar, below **New chat**. Its tooltip reads "Work with your team".
- If you use Slack, the main ideas are the same. A workspace holds channels, and you post messages and files in a channel. Crew adds two things. Teams group channels. Agents can read and post in a channel when you allow it.
- A Crew workspace runs on a Linux server that your lab uses, under the host's own account. Your computer reaches it with SSH (Secure Shell), the standard way to sign in to a server over a network. Crew uses your own account on that server.
- You see only the channels you are in. Crew has no list of other channels to browse. To get into a channel, ask its owner or the host to add you.
- Biorouter and the workspace check every action you take. When the workspace refuses an action, Crew tells you why in a sentence.
- The desktop app and the `biorouter crew` command line use the same background service on your computer. Switching between them does not create a second workspace. See [Command line](command-line.md).

## What you need

| You need | Why | Who usually provides it |
|---|---|---|
| The Biorouter desktop app | Crew is a view inside the app. You can also use Crew from a terminal with the `biorouter crew` commands. See [Command line](command-line.md). | You, or your IT team |
| An account on your lab's Linux server | Crew connects with your own username on that server. | Your IT team, or whoever runs the server |
| A way to sign in to that account | An SSH key, a password, a verification code, or a mix of these. | Your IT team |
| Crew installed for your account on the server | Each account needs its own copy of the `biorouter-crew` program at `~/.local/bin/biorouter-crew`. The host's account needs one too. | Your host or your IT team |
| The server already verified on your computer | Crew connects only to servers whose identity (their SSH host key) is already in your computer's known hosts file. Crew never accepts a new server identity by itself. | You, with the fingerprint from your IT team |
| A workspace to join, or the plan to host one | A workspace exists only after one member hosts it. To join, you need the invitation message from the host. An invitation lasts 24 hours. | The host |
| Your institution's short ID, for a Private workspace | A short lowercase ID such as `ucsf` or `sdsc`. The invitation usually fills it in for you. | The host |

When one of these is missing, Crew shows a message that names the problem. The table in [What the main area can show](#what-the-main-area-can-show) lists each message and what to do.

> **Note.** Crew has been tested with the Biorouter desktop app on macOS. It has not been tested on Windows or Linux desktops.

### Signing in to the server

When the server asks for a password or a verification code, Crew opens a window titled "Sign in to {server}". It reads "Type your password or verification code in the box below. Nothing you type is saved."

Choose **Trouble signing in?** in that window to see three tips:

- "Use the same username and password you use for this server."
- "If your IT team gave you a jump host, add it in Connection settings."
- The path of the known hosts file that Crew checks servers against, with a button to copy it.

The Sign in window works only in the desktop app. If you open Crew in a web browser (with `biorouter serve`), Crew shows "Signing in needs the Biorouter desktop app." Sign in from the desktop app, or run `biorouter crew auth` in a terminal. See [Command line](command-line.md).

[Connections and troubleshooting](connections-and-troubleshooting.md) covers signing in step by step.

#### Jump hosts

A jump host is a server you pass through to reach the lab server. You need one only if your IT team gives you one. Where you type it depends on when:

| When | Where |
|---|---|
| You join a workspace | **Jump host**, under **Advanced** in the Join dialog. Separate several host names with commas. |
| You host a workspace | **Jump hosts**, under **Advanced** in the Host dialog. |
| Later | **Jump hosts**, in **Connection settings…** in the workspace menu. That window exists only after you join or host. |

Each jump host also needs settings in your SSH settings file (usually `~/.ssh/config`). Crew checks them before it connects. Your IT team adds them. [Administration](administration.md#ssh) lists them. Without them, Crew does not connect.

Crew also refuses to connect when your SSH settings reach the server through a custom `ProxyCommand` instead of a jump host, or when they pass on your Kerberos credentials (`GSSAPIDelegateCredentials yes`). Ask your IT team to change those settings.

### The approval secret

Biorouter runs a background service on your computer. The desktop app and the command line both use it. The service accepts actions that need a person's approval only from someone who knows your approval secret.

On macOS and Linux, Biorouter asks for this secret every time you open it, before its window appears. Which window you see depends on whether the service is already running:

| Situation | Window title | What to type |
|---|---|---|
| The service is not running. This is the case the first time you open Biorouter, and after you restart your computer. | "Set approval secret for shared BioRouter daemon", then "Confirm shared daemon approval secret" | A secret you choose, twice. |
| The service is already running. Quitting the desktop app leaves it running, so this is the case every time you reopen Biorouter until you restart your computer. It is also the case after you start the service from the command line. | "Connect to existing BioRouter daemon" | The secret the service started with. |

Each of these windows has two buttons, **Cancel** and **Continue**. On Linux, **Continue** reads **OK**. Each window waits three minutes. After that, Biorouter stops waiting and shows an error.

To set a secret:

1. Before you open Biorouter, create the secret in your password manager. Make it 32 to 4096 characters long. Use English letters, numbers and punctuation marks. Do not use spaces. Biorouter does not save the secret and cannot show it to you again.
2. In the window titled "Set approval secret for shared BioRouter daemon", type the secret. Then choose **Continue**, or press Return.
3. In the window titled "Confirm shared daemon approval secret", type the same secret again. Then choose **Continue**, or press Return.

To connect to a service that is already running, type the secret the service started with in the window titled "Connect to existing BioRouter daemon". Then choose **Continue**, or press Return.

Use the same secret every time Biorouter asks you to set one. Then you have only one secret to remember.

The approval secret is not your computer login password, your SSH password, or your Crew vault passphrase. Use a different secret.

If something goes wrong, Biorouter shows a window titled "Biorouter Error". Its text starts "Failed to create main window: Error:" and continues with one of the messages below. When you close that window, Biorouter closes too. Open Biorouter again and type the secret.

| Message | What it means | What to do |
|---|---|---|
| A message that starts "Approval secret must contain" and names the allowed length and characters | The secret has fewer than 32 characters, or it has a space or a character that is not an English letter, a number or a punctuation mark (an accented letter, for example). Any of the three windows can show this. | Open Biorouter again and type a secret that follows step 1. |
| "The secure response exceeds the allowed length." | The secret has more than 4096 characters. | Open Biorouter again and type a shorter secret. |
| "The native secure prompt timed out after three minutes. Reopen the app and complete the password dialog to continue." | A secret window stayed open for three minutes without **Continue**. | Open Biorouter again with the secret ready in your password manager. |
| "Approval secrets did not match. No daemon was started. Reopen the app to try again." | The two secrets you typed differ. | Open Biorouter again, then type the same secret twice. |
| "Shared daemon startup cancelled. No daemon was started. Reopen the app when ready to supply your approval secret." | In "Set approval secret for shared BioRouter daemon", you chose **Cancel**, or chose **Continue** with the field empty. | Open Biorouter again when you have the secret ready. |
| "Shared daemon startup cancelled. No daemon was started." | In "Confirm shared daemon approval secret", you chose **Cancel**, or chose **Continue** with the field empty. | Open Biorouter again. You type the secret twice from the start. |
| "Daemon attachment cancelled. Reopen the app and supply the existing approval secret to connect." | In "Connect to existing BioRouter daemon", you chose **Cancel**, or chose **Continue** with the field empty. | Open Biorouter again and type the secret the service started with. |
| "The daemon did not accept human-authorized access. Reopen the app to retry with its existing approval secret, or cancel attachment." | The secret you typed is not the one the service started with. | Open Biorouter again and type the right secret. |

If you forget the secret while the service is running, the simplest way out is to restart your computer. The restart stops the service. The next time you open Biorouter, it asks you to set a new secret.

`biorouter crew daemon stop` needs the secret, but you can end the service from a terminal without it and without restarting. See [If you forget the approval secret](command-line.md#if-you-forget-the-approval-secret).

Either way, check your agent tasks, file transfers and chat access afterward:

1. Open Crew and choose **Connect to {workspace}** in the main area.
2. In the Agents section of the Crew sidebar, look for tasks that read "Interrupted", "Outcome unknown" or "Stop not confirmed". Choose a task to go to it in its channel. [The task row](agents-and-chat-access.md#the-task-row) says what to do for each word.
3. In each channel you used, choose **Channel details** at the right end of the channel header, then the **Files** tab. Under "In progress", look for transfers that read "Paused" or "Not confirmed". [Transfer states](messages-and-files.md#transfer-states) says what to do for each word.
4. Open the workspace menu (choose the workspace name at the top of the Crew sidebar) and choose **Agent access…**. Rows that read "Stopped on this device" are chats you revoked that the workspace has not confirmed yet. Crew confirms them by itself. See [Rows in an Agent access list](agents-and-chat-access.md#rows-in-an-agent-access-list).

To make the same checks from a terminal, see [Recover after a restart](command-line.md#recover-after-a-restart).

### Where Crew keeps your keys

Crew keeps a key on your computer that proves which computer is yours. By default it stores that key in your system keychain. You do not need to set anything up.

If you prefer, you can keep the key in an encrypted vault that opens with a passphrase you choose. Open the You menu at the bottom of the Crew sidebar and choose **Keys and security…**. When the vault is locked, the connection bar shows "Your Crew vault is locked." with an **Unlock** button. [Privacy and security](privacy-and-security.md) explains the vault.

## The first screen

The first time you open Crew, it shows the welcome screen. The screen has no Crew sidebar yet. It shows:

- the title "Work together in Crew" and the line "Chat, share files and run agents with your lab."
- a button, **Join a workspace**
- a link under it, **Host a new workspace**

Choose **Join a workspace** when someone in your lab sent you an invitation message. Choose **Host a new workspace** when you are setting up your lab's workspace on a server and will invite others. Most people join. Only one person, the host, creates a workspace.

While Crew loads your saved workspaces, it shows placeholder shapes in place of the sidebar and the messages.

### Joining, in brief

1. Send your host your exact username on the lab server before they invite you. It is the name before the @ when you sign in with SSH, for example `bob` in `ssh bob@lab.example.edu`. A name that is close is not enough. See [Before you start](joining-a-workspace.md#before-you-start).
2. Verify the server on this computer before you paste the invitation. You need the server fingerprint from your IT team. See [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer).
3. Ask your host for the invitation message. It lasts 24 hours.
4. Choose **Join a workspace**.
5. Paste the whole message into **Invitation from your host**.
6. Choose **Join {workspace}**. Crew shows your code under "Send {host} this code:".
7. Send the code to your host the way you usually reach them, for example by email or Slack.
8. Wait for your host to let you in. While you wait, Crew shows "You can close Biorouter: {host} can still let you in with the same code. Next time, open Crew and connect to {workspace}."

Your code is not the fingerprint. The Join dialog also shows a fingerprint under **Check this invitation (optional)**. Do not send the fingerprint to your host. [Joining a workspace](joining-a-workspace.md) covers every step.

### Hosting, in brief

The **Host a new workspace** dialog has three steps: "Name", "Start" and "Create".

1. In the "Name" step, type a **Workspace name** ("Lowercase letters, numbers and dashes.") and **Your server login**, for example `alice@hpc.example.edu`.
2. In the "Start" step, you start Crew on the server. Choose **Start it for me**. Biorouter signs in to the server with your SSH settings and runs the start commands for you. If the server asks for a password or a code, run the commands yourself in a terminal instead.
3. In the "Create" step, choose **Create workspace**. This computer becomes the workspace's first admin device.

[Hosting a workspace](hosting-a-workspace.md) covers every step, including inviting people and letting them in.

### Adding another workspace later

You can save more than one workspace on the same computer. Open the workspace menu (the workspace name at the top of the Crew sidebar), choose **Add a workspace**, then choose **Join a workspace…** or **Host a new workspace…**.

## What the main area can show

The main area to the right of the Crew sidebar shows one screen at a time. Outside a channel, it is one of these.

| You see | What it means | What to do |
|---|---|---|
| "Work together in Crew" | This computer has no saved workspace. | Choose **Join a workspace** or **Host a new workspace**. |
| Placeholder shapes | Crew is loading. | Wait. |
| "Connecting to {server}…" | Crew is connecting to the server. | Wait. |
| "Sign in to {server}" and "The server needs your password or a verification code." | The server wants your password or a code. | Choose **Sign in**. |
| "{workspace} is offline" and "Connect to see your channels." | You are not connected. | Choose **Connect to {workspace}**. |
| "Crew isn’t set up for your account on {server}" | The `biorouter-crew` program is missing from your account on the server. | Ask your host or IT team to install it, then choose **Try again**. |
| "Can’t verify {server} yet" | Your computer does not know this server's identity yet. | Get the server's fingerprint from your IT team and add the server to your known hosts file, then choose **Try again**. |
| "{server}’s identity changed" | The server's identity differs from the one your computer knows. | Do not connect. Choose **Copy details for IT** and send the details to your IT team. |
| "This isn’t the workspace you joined" | The server answered with a different workspace. | Do not continue. Choose **Copy details** and ask your host what changed. |
| "Send {host} this code:" | You joined, and the host has not let you in yet. | Send the code to your host and wait. |
| "You’re in {workspace}" and "Ask {host} to add you to a team." | You are a member, but you are in no team yet. | Ask the host to add you to a team, or choose **Create a team**. |
| "You’re invited to {team}" | Someone invited you to a team. | Choose **Join {team}**. |
| "Get {workspace} ready" | You are the host, and the workspace has no teams yet. | Follow the setup checklist. |
| "No open channels in {team}" and "Create one to start talking." | The team has no open channels. | Choose **Create channel**. |
| "Messages will show here again once live updates are back." | Crew stopped receiving live updates. | Choose **Retry** in the connection bar. |

After a failed attempt to connect, the offline screen adds a line such as "Tried again at {time}. Couldn’t reach {server}." When the network is the problem, it also says "Crew keeps trying by itself while the network is down."

If Crew says the workspace is offline but you expect it to be connected, choose **Connect to {workspace}**. [Connections and troubleshooting](connections-and-troubleshooting.md) covers each of these screens in detail.

## A tour of the Crew view

Once you are connected to a workspace, the Crew view has up to four columns.

```text
+============+==================+====================================+================+
| Biorouter  | Crew sidebar     | Channel                            | Details pane   |
| sidebar    |                  |                                    | (when open)    |
|            | lab           v  | #methods v  Restricted   2 chats   | About          |
| Home       | o Connected      | (connection bar, when needed)      | Members        |
| New chat   |   Private . UCSF |                                    | Files          |
| Crew       |                  | Welcome to #methods                | Agent access   |
|            | Invitations      | ...messages...                     |                |
|            | Analysis Lab     |                                    |                |
|            |   # general      |                                    |                |
|            |   # methods      |                                    |                |
|            | + Add channel    | +================================+ |                |
|            | + Add team       | | Message #methods               | |                |
|            | Agents           | | Attach  Ask my agent     Send  | |                |
|            | Alice Chen    v  | +================================+ |                |
+============+==================+====================================+================+
```

- The Crew sidebar lists your workspace, its status, your teams and channels, your agents, and you.
- The channel column shows the open channel: its header, the connection bar, the messages and the message box.
- The details pane opens on the right side when you ask for it. It shows facts about the channel.

### The Crew sidebar

From top to bottom, the Crew sidebar holds:

1. The workspace name, which opens the workspace menu.
2. The status row: a connection status word on the left and the privacy chip on the right.
3. Sections that need your attention, when there are any.
4. Your teams, each with its channels.
5. The Agents section, when there is something to show.
6. The You row, fixed at the bottom.

While you wait for the host to let you in, the sidebar shows "Your channels appear here once {host} lets you in." in place of teams.

#### The workspace menu

Choose the workspace name at the top of the sidebar to open the workspace menu. When a name is too long to fit, point at it to see the whole name.

The top of the menu shows:

- the workspace name
- "Hosted by {host}"
- "Signed in as @{username} on {server}", once Crew has verified your connection
- the connection status
- "Fingerprint" followed by 16 characters, with a **Copy** button, while you are connected

The host reads this fingerprint to people who ask to check their invitation.

| Item | What it does | When it appears |
|---|---|---|
| **Invite people to {workspace}…** | Invites someone to the workspace. | For the host only. |
| **People…** | Opens workspace settings at the People tab. | Always. |
| **Privacy…** | Opens workspace settings at the Privacy tab. | Always. |
| **Agent access…** | Opens workspace settings at the Agent access tab. | Always. |
| **Create team…** | Creates a team. | Always. |
| **Reconnect** | Tries the connection again. | Only when you are not connected. |
| **Sign in…** | Opens the Sign in window. | Only when the status is "Sign-in needed". |
| **Disconnect** | Closes the connection to the workspace. | Always. |
| **Connection settings…** | Opens this computer's settings for the workspace. | Always. |
| One item per saved workspace, under "Switch workspace" | Switches to that workspace. | When you have two or more saved workspaces. |
| **Add a workspace** | Opens **Join a workspace…** and **Host a new workspace…**. | Always. |

**Invite people to {workspace}…** (host only), **People…**, **Privacy…**, **Agent access…** and **Create team…** stay unavailable until your connection is verified. A note above them says why: "Available after you join", "Available once you’re connected" or "Available once the connection is verified".

Workspace settings opens in a window titled "{workspace} settings" with four tabs: **General**, **People**, **Privacy** and **Agent access**. In the People tab, the host carries a "Host" badge.

#### The status row

The status row sits under the workspace name. The word on the left tells you the state of your connection.

| Status | What it means | What to do |
|---|---|---|
| "Connected" | Crew is connected and has verified the workspace's identity. A screen reader hears "Connected · identity verified". | Nothing. |
| "Connecting…" | Crew is connecting. | Wait. |
| "Reconnecting…" | The connection dropped, and Crew is picking it up again. | Wait. |
| "Checking connection" | Crew is connected and waiting for the first view of the workspace. | Wait. |
| "Updating…" | Crew is checking the workspace again. The last view stays on screen, dimmed, until the check finishes. | Wait. |
| "Not joined yet" | You are connected, but the host has not let you in. The tooltip reads "Waiting for {host} to let you in". | Check that the host has your code. |
| "Updates unavailable" | Crew is connected but not receiving live updates. | Choose **Retry** in the connection bar. |
| "Sign-in needed" | The server wants your password or a verification code. | Choose **Sign-in needed**. It opens the Sign in window. |
| "Not set up on this server" | The `biorouter-crew` program is missing from your account on the server. | Ask your host or IT team to install it. |
| "Can’t verify server" | Crew could not confirm the server's identity, so it does not connect. | Read the message in the main area, and ask your IT team to confirm the server's identity. |
| "Offline" | You are not connected. This is normal after your computer restarts, after you choose **Disconnect**, after you save a change in **Connection settings…**, and after your computer sleeps or loses the network. In that last case, Crew tries to reconnect by itself for up to an hour. | Choose **Connect to {workspace}** in the main area. After your computer slept or lost the network, you can also wait. See [Automatic reconnection](connections-and-troubleshooting.md#automatic-reconnection). |
| "Can’t connect" | The last attempt to connect failed. | Choose **Connect to {workspace}** in the main area. The connection bar says why the last attempt failed. |

#### The privacy chip

The privacy chip sits at the right end of the status row. It shows the privacy that applies to you in this workspace:

- "Private · {institution}" with a padlock, for example "Private · UCSF"
- "Public"

Privacy in Crew decides which AI models may read the workspace. It does not decide which people can see it. Only people the host lets in can see a workspace.

While Crew checks your privacy, the chip reads "Checking privacy…". In most other states, such as "Offline", "Reconnecting…" or "Not joined yet", the chip is hidden. That is expected.

Choose the chip to open a panel that explains your privacy. It shows a summary such as "Only private and UCSF-approved models can read lab.", then three facts: "Your connection", "Workspace" and "Institution". It also says why the privacy is what it is. [Privacy and security](privacy-and-security.md) explains Private and Public in full.

#### Sections that need your attention

These sections appear only when they have something in them.

| Section | Who sees it | What it holds |
|---|---|---|
| "Invitations" | Anyone | Team or channel invitations for you. Each has a **Join** button. |
| "Waiting to join" | The host | People invited to the workspace. Each has a **Let in…** button for when they send their code. |
| "Joined, not in your teams" | The host | People who joined the workspace but are in none of your teams. Each has an **Add to a team…** button. |

#### Teams and channels

- Each team has a header with its name. Choose the header to collapse or expand the team. A collapsed team still shows the channel you have open.
- Point at a team header to show two buttons: **+** creates a channel in that team, and **⋯** opens the team menu.
- The team menu offers **Members of {team}…**, **Create channel…**, **Add people to {team}…** (for the team's owner or the host), **Rename team…** (for the team's owner, where the workspace supports it) and **Copy for support**.
- A team owner sees "· {n} invited" in the header when people invited to the team have not accepted yet.
- Each channel row shows `#` and the channel name. A channel with unread messages shows its name in bold with a count. The count stops at "99+".
- A pencil icon on a channel row means you have unsent text there.
- **Add channel** sits under each team. **Add team** sits after the last team.
- "Archived ({n})" appears under a team that has archived channels. Choose it to show them.
- If you do not own a team, you see "Other channels in {team} appear once someone adds you."
- Right click a channel row, or press Shift+F10, to open its menu: "Mark as read" (for the open channel), "Copy channel name" and "Copy channel ID".

[Teams, channels and people](teams-channels-and-people.md) covers creating and managing teams and channels.

#### The Agents section

The Agents section lists two kinds of rows:

- Your agent tasks that have not finished, for example "#methods · Working…". A task waiting for your approval carries a "Needs you" badge. Choose the row to go to the task in its channel.
- Your ordinary Biorouter chats that can read and post in this workspace, for example "Plot review · #methods". Choose the row to open Chat access, where you can revoke the chat's access. This works even while the workspace is offline.

To also see revoked chats and finished tasks, choose **⋯** next to "Agents" and turn on **Show revoked and finished**. [Agents and chat access](agents-and-chat-access.md) covers agents in full.

#### The You row

The You row sits at the bottom of the sidebar. It shows your picture, your display name, your `@username` and "on {server}". Before the workspace names you, it shows your server login instead, such as `crew_alice@lab-server`.

Choose the You row to open the You menu:

| Item | What it does |
|---|---|
| **Edit profile…** | Changes your display name. Available once your connection is verified. |
| **Keys and security…** | Shows where Crew keeps your keys and which devices are on your account. |
| **Copy my username** | Copies your username. |

### The channel header

The channel header runs across the top of the channel column. From left to right:

1. The channel name with a small arrow, for example `#methods`. Choose it to open the channel menu.
2. A badge, "Restricted" or "Public-safe". Choose it to open the channel's About tab.
   - "Restricted": "Only private models can read it. It doesn’t limit who’s in the channel."
   - "Public-safe": "Public models may read it when the workspace allows."
3. An "Archived" badge, for an archived channel only.
4. "Up to date", shown for a moment after you choose **Refresh channel**.
5. On the right side:
   - a count such as "2 chats", "1 task" or "3 agents", when your chats or agent tasks can post here. Choose it to open the **Agent access** tab.
   - the pictures of up to three members, with the member count. Choose them to open the **Members** tab.
   - the **Channel details** button, which opens and closes the details pane.

The channel header does not show privacy. Privacy appears only in the status row.

### The channel menu

| Group | Items |
|---|---|
| Views | **Channel details**, **Members**, **Files**, **Agent access** |
| Actions | **Add people…** (channel owner), **Mark as read**, **Refresh channel**, **Copy channel name** |
| Owner actions | **Rename…**, **Transfer ownership…**, **Archive channel…** (channel owner only) |
| Support | **Copy for support**, which holds **Copy channel ID** |

People who do not own the channel do not see the owner items. **Rename…** appears only where the workspace supports renaming channels.

### The connection bar

The connection bar sits directly under the channel header, or at the top of the main area on other screens. It is empty when everything works. When something needs you, it shows one note of each kind:

- Live updates stopped. Choose **Retry**.
- An action or a connection attempt failed, with the reason in words, such as "Can’t reach {server}." Choose **Try again**, **Dismiss** or **Connection settings…**. While the offline screen shows its own **Connect to {workspace}** button, the bar has no **Try again**.
- Something that needs you: "Your Crew vault is locked." with **Unlock**, or "Reconnecting to {workspace}…" while Crew reconnects.
- "A new device was added to your account on {date}." Choose **Review** to see your devices.

When an error comes from a part of the window you cannot see, it appears here.

### The messages

- A channel starts with "Welcome to #{channel}" and the name of the person who created it.
- Dividers mark "Today" and "Yesterday". A line labeled "New" marks where your unread messages start.
- Posts by agents carry an "Agent" badge. Your own agent appears as "Your agent".
- A message you sent shows "Sending…" until the workspace delivers it.
- When you scroll up, a button above the message box takes you back to the newest messages. It reads **Jump to latest**, or a count such as **3 new messages** when messages arrived while you were scrolled up.

[Messages and files](messages-and-files.md) covers reading and writing messages.

### The message box

The message box sits at the bottom of the channel column. Its placeholder reads "Message #{channel}".

| Control | What it does |
|---|---|
| **Attach** | Opens **Upload a file…** and **Share a server path…**. |
| **Ask my agent** | Opens a form to give your agent a task in this channel. The task text is posted in the channel so everyone there can see it. |
| Send | Sends the message. Its tooltip reads "Send message". |

- Press Enter to send. Press Shift+Enter to start a new line.
- Your text stays in the box until the workspace accepts it. If sending fails, Crew shows "Couldn’t send." and the reason above the box.
- You can also drop a file onto the channel, or paste it. Biorouter asks you to confirm each file. Files can be up to 1 GB. Nothing reaches the channel until you send: Crew shows "Press Send to share it."
- In an archived channel the box reads "This channel is archived." While Crew checks your access, it reads "Verifying access…".

### The details pane

Choose **Channel details** in the channel header to open the details pane on the right side. It has four tabs:

| Tab | What it shows |
|---|---|
| **About** | The channel's name, who can read it, its owner, who created it, and its team. |
| **Members** | Everyone in the channel. The channel's owner carries a "Channel owner" badge. |
| **Files** | Files shared in the channel and files you are uploading. |
| **Agent access** | Your chats and tasks that can read and post in this channel. |

The same pane also opens for **Ask my agent** and for Chat access. It shows one of these at a time.

- To close the pane, choose its close button or press Escape while you are in the pane.
- When the window is narrow, the pane covers the messages. Its header then shows **Back to #{channel}** to return.
- The pane stays open when you refresh. It closes when you switch channels or workspaces.

### Crew outside the Crew view

When you connect an ordinary Biorouter chat to a Crew channel, that chat shows a **Crew · #{channel}** chip. Choose the chip to manage the chat's access. The full connection status appears only inside the Crew view.

### Where Crew keeps your place

- Crew reopens each workspace on the channel you last had open.
- Unsent text stays in each channel's message box while the app is open. Crew does not save it to disk, so it is gone after you quit Biorouter.
- When you return to Crew, it shows the last view, dimmed, while it checks the workspace again.

### Finding internal IDs for support

Crew does not show the internal IDs of people, teams, channels or messages. If a support person asks for one, look for **Copy for support** in the menu for that person, team, channel or message. The About tab of a channel holds its ID under **IDs for support**.

## Using the keyboard

| Where | Keys | What happens |
|---|---|---|
| Crew sidebar | Up and Down arrows | Move between teams and channels. |
| Crew sidebar | Home and End | Move to the first or last row. |
| Crew sidebar | Left and Right arrows | Collapse or expand a team. On a channel row, Left moves to its team header. |
| Crew sidebar | Tab | Leaves the list. From a team header, Tab reaches its **+** and **⋯** buttons. |
| Channel row | Shift+F10, or the Menu key | Opens the channel row's menu. |
| Messages | Up and Down arrows | Move between messages. |
| Messages | Home and End | Move to the first or last message. |
| Messages | Tab | Reaches the actions of the selected message. |
| Message box | Enter | Sends the message. |
| Message box | Shift+Enter | Starts a new line. |
| Details pane | Escape | Closes the pane. |
| Any menu | Enter or Space, arrows, Escape | Open the menu, move through it, close it. |

When you come back to the sidebar with Tab, focus lands on the channel you have open.

## Glossary of Crew terms

| Term | Meaning |
|---|---|
| Agent | An AI assistant in Biorouter. It can read and post in a channel when you allow it. Its posts carry an "Agent" badge. |
| Agent access | The list of your chats and agent tasks that can read and post in a channel. The channel menu, the details pane and workspace settings all use this name. |
| Agent task | Work you give your agent with **Ask my agent**. The task text is posted in the channel. The task's access ends when the task finishes. |
| Approval secret | A secret you choose when Biorouter starts its background service. It is not your computer password, SSH password or vault passphrase. |
| Background service | The Biorouter program that runs on your computer and does the work for the desktop app and the command line. Its technical name is the daemon. |
| Channel | A place for messages and files inside a team, shown as `#methods`. You see only the channels you are in. |
| Channel owner | The person who manages a channel. The owner adds people, renames the channel where the workspace allows it, transfers ownership and archives the channel. Shown with a "Channel owner" badge. Not the same as the host. |
| Chat access | Permission for one of your ordinary Biorouter chats to read and post in a channel. You connect a chat by typing `/crew` in it. "Access ends when you revoke it, or after an hour." |
| Connection | This computer's saved way to reach a workspace: the server, your username and your privacy choice. It is saved on this computer only. You change it in **Connection settings…**. |
| Details pane | The panel on the right side of a channel, with the **About**, **Members**, **Files** and **Agent access** tabs. |
| Fingerprint | A short value you compare with a trusted copy to confirm an identity. The workspace fingerprint identifies the workspace. A server fingerprint identifies the server. Neither one is your code. |
| Host | The member who runs the workspace on the server under their own account. Only the host invites people to the workspace, lets them in, and changes the workspace's privacy. The People tab shows a "Host" badge. |
| Institution | A short lowercase ID for your organization, such as `ucsf`. A Private workspace uses it to decide which AI models are approved. |
| Invitation | Either the message the host sends you to join the workspace (it lasts 24 hours), or a team or channel invitation that appears in the sidebar with a **Join** button. |
| Known hosts file | The file on your computer where SSH keeps the identities of servers you have verified. Crew connects only to servers listed in it. |
| Member | A person the host let into the workspace. Crew shows members by display name and `@username`, which is their username on the server. |
| Privacy | Private or Public. It decides which AI models may read the workspace, not which people can see it. Your connection and the workspace each have a setting. Public applies only when your connection is Public and the workspace allows Public. |
| Public-safe | A channel label. "Public models may read it when the workspace allows." |
| Restricted | A channel label. "Only private models can read it. It doesn’t limit who’s in the channel." |
| Server | The shared Linux computer where the workspace runs. When your SSH settings give the server a short name, Crew uses that name. |
| SSH | Secure Shell, the standard way to sign in to a server over a network. Crew uses it to reach the workspace with your own server account. |
| Status row | The line under the workspace name that shows your connection status and the privacy chip. |
| Team | A group of people and channels inside a workspace. Team names are unique in a workspace. Every new team comes with a `#general` channel. |
| Vault | An optional encrypted store for Crew's keys on your computer, unlocked with a passphrase. Without it, Crew uses your system keychain. |
| Workspace | Your lab's shared space in Crew. It holds teams, channels and people, and runs on a Linux server under the host's account. |
| Your code | 16 letters and numbers that Crew shows after you choose Join, in the form `XXXX-XXXX-XXXX-XXXX`. You send it to the host, who enters it with **Let in…**. It stays the same if you reconnect, disconnect or close the app. It never contains the letter U. |

## Related documentation

- [Crew user manual](README.md): the list of every page in this manual.
- [Joining a workspace](joining-a-workspace.md): paste your invitation, send your code, and wait to be let in.
- [Hosting a workspace](hosting-a-workspace.md): create a workspace, invite people and let them in.
- [Teams, channels and people](teams-channels-and-people.md): create teams and channels and manage who is in them.
- [Messages and files](messages-and-files.md): read and write messages and share files.
- [Agents and chat access](agents-and-chat-access.md): give your agent tasks and connect your chats to a channel.
- [Privacy and security](privacy-and-security.md): Private and Public, institutions, keys and fingerprints.
- [Connections and troubleshooting](connections-and-troubleshooting.md): what each status and error message means and what to do.
- [Command line](command-line.md): the `biorouter crew` commands.
- [Administration](administration.md): server requirements and installing `biorouter-crew` for each account.
- [Installation and setup](../getting-started/installation.md): install the Biorouter desktop app, which includes Crew.
