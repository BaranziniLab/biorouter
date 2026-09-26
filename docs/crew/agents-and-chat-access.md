# Agents and chat access

> **What this is.** How to give an AI agent access to a Crew channel with **Ask my agent** or `/crew`, and how to follow, revoke and restore that access.
> **Status:** Current.
> **Audience:** Lab members who use Crew in the Biorouter desktop app.

A task is one job for your agent. A connected chat is a Biorouter conversation that reads a channel and posts there while you talk to it. Nothing reads or posts until you allow it, access lasts one hour at most, and everyone in the channel sees what your agent posts.

## Before you start

- Set up at least one AI model in Biorouter. See [Choosing a model provider](../getting-started/choosing-a-model-provider.md).
- Your agent runs on this computer with your model. Keep the computer on until the task shows "Done". On a Mac or Linux, quitting Biorouter does not stop a task. Quitting on Windows, or restarting or shutting down the computer, stops it.
- One computer runs at most four tasks at once. A task stops after 20 model turns or 60 tool calls, so split long work into several tasks.
- Access lists show only your own chats and tasks, never other people's agents.

Workspaces and connections start Private, so you usually need a model approved for your institution, or a local model such as Llama Server or Ollama. A public model, such as one used with your own OpenAI key, is refused. A Private workspace or connection, or a Restricted channel, needs the following. See [Privacy and security](privacy-and-security.md).

1. The host has set the workspace's institution. See [Hosting a workspace](hosting-a-workspace.md).
2. A Private connection has an institution, set in **Connection settings…**.
3. The model is approved for that institution, or is local.

Crew checks the first two when you choose the start or Allow button. A model is approved when its provider carries your institution, and you cannot approve one yourself. To add such a provider, open **Settings** > **Models** > **Configure providers**, then the **Institutional** tab.

## Start a task with Ask my agent

1. Open the channel that should get the result. Text already in the message box is copied into the task.
2. Choose **Ask my agent**, the robot icon left of the send button. An archived channel, or one Crew is still verifying, has no such button.
3. Check the pane's first line, such as "Posts to #methods in Analysis Lab · lab".
4. Type the task in **Task**. Enter adds a new line.
5. Check **Model**, or choose **Change**. See [Choose a model](#choose-a-model).
6. Optionally open **Advanced**. See [Also read other channels](#also-read-other-channels).
7. Read any notes, then choose **Start my agent and allow posting here**.

The pane closes, and Crew highlights your task in the channel. Escape or × closes the pane without starting.

Crew posts your whole task in the channel as "Task: …", so do not paste data you would not post there yourself. You see the agent's posts as "Your agent". Others see "Alice Chen's agent".

### Choose a model

If the pane reads "No models are set up.", choose **Open Settings** and add one. Otherwise choose **Change** or "Choose a model", and type in **Search models** to filter. For an unlisted model, type its exact name and choose "Use “name” with provider".

| Chip | Meaning |
|---|---|
| "Private · UCSF" | Approved for the institution named |
| "Private · On this machine" | Local, accepted everywhere |
| "Private · No stated institution" | Refused when the task needs a Private model |
| "Public" | Refused for Restricted channels and for Private workspaces or connections. With a remote work folder it can run, but cannot use the folder. |
| "Not approved for UCSF" | Crew will not start with it |

Claude Code and Codex appear in the list but are always refused.

### Also read other channels

When there is something to choose, the pane shows **Advanced**. Under "Also read", tick up to 16 more channels for the agent to read. It still posts only here. If your connection has a **Remote work folder** (set in **Connection settings…**), **Advanced** also shows "Can read {folder}", or "Can run commands in {folder}" when **Let my agent run commands in this folder** is on.

### Files named in the task

Crew notices names ending in .csv, .tsv, .xls, .xlsx, .json, .txt, .h5ad or .parquet. Put a name with spaces alone in double quotes, as in "Plate Reader.csv". Names in web links do not count.

- "No file named results.csv is shared in #methods. …": fix the spelling or [share the file](messages-and-files.md). If you start anyway, the agent says what it used instead.
- "2 files named gina-assay.csv are shared … newest one …": to use an older copy, say in the task who shared it and when.

### If Crew cannot tell whether a task started

If the connection drops as you start, starting again could run the task twice. The pane shows "Inspect the previous task before starting again" and blocks the start button.

1. Choose **Show task in channel** or **Open chat history**, and check what the task did.
2. Open **Ask my agent** and tick **I checked the previous task and its effects.**
3. Check **Task**, then choose **Start a new task**.

Quitting Biorouter or reloading the window also clears the warning.

## Follow a task

Each task gets a row in the channel. A normal task shows "Starting…", then "Working…", then "Done". These statuses need you:

- "Waiting for your approval": choose **Review** and answer in the task's conversation. The Agents section marks it "Needs you". Crew approves nothing itself.
- "Stop not confirmed": choose **Try stopping again**.
- "Interrupted" or "Outcome unknown": choose **Open**, and check the conversation and the channel before you start again.
- "Couldn’t finish": choose **Open** to see why, or copy the reason with ⋯ > **Copy error**.

**Stop** asks "Stop your agent?". Choose **Stop task**, and the row reads "Stopping…", then "Stopped". Work already done and posts stay. ⋯ (More task actions) holds **Show in chat history**, **Copy error** and **Copy for support** > **Copy task ID**.

### Read the result

The agent's reply is posted as the result, and its first line names the file used. Crew adds a last line from what the task read. Trust it over the agent's words.

| The task read | Last line |
|---|---|
| One file | ``Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).`` |
| The newest of several files with one name | The same, plus when it was shared |
| Several files | "Sources: " and each file |
| An older copy | "(earlier copy)" and ``A newer copy of `gina-assay.csv` was shared and was not read.`` |
| Nothing | "No shared file was read for this result." |

"Task finished without a text result." means no reply. "(This reply was shortened to fit the channel.)" means a long reply was cut. If the agent used the wrong file, share it or name the copy, then start a new task.

### The task's conversation

**Open** shows the task's own conversation, titled "Crew · #methods · " and the task's first line. It may be missing from "Recents", so use **Open** or **Show in chat history**. A task uses only Crew tools and the checklist tool, the task list the agent keeps while it works.

A task's access ends once it posts its result, and its access row reads "Ended". This is normal. The conversation offers only **Start a new chat**. For more work, start a new task.

## Connect a chat with /crew

The chat needs a sent message (otherwise you see "Start the chat first"), no attached files, images or reference chips (otherwise "Draft kept"), a model, and no reply in progress. Reference chips are labels added with @, or with **Quote it** in the menu when you right click selected text in a chat.

1. Type `/crew` alone in the chat's message box and press Enter. Nothing is sent to the model.
2. Crew opens the first workspace in its list, at the channel you last had open there, with the Chat access pane. If you see "{workspace} is offline", choose **Connect to {workspace}**.
3. Check the channel in the pane. It is always the channel Crew shows. To change it, see [Another channel or workspace](#another-channel-or-workspace).
4. Optionally open **Advanced** and tick more channels under "Also read".
5. Choose **Allow “Plot review” to read and post in #methods** (**Allow this conversation to read and post here** when the title is unknown).
6. The pane shows "Connected." and "Active · ends 4:40 PM". Choose **Back to chat**, and ask for what you need.

### Another channel or workspace

This works only for a chat never connected before.

1. For another workspace, choose the workspace name at the top of the Crew sidebar, then the workspace under "Switch workspace". Connect it if it is offline.
2. Choose the channel in the Crew sidebar.
3. On the note "Connect “Plot review” to #imaging?", choose **Review access**, then continue from step 4.

To change such a chat's model, choose the model name under its message box (brain icon), then **Change model**. Pick an approved or local model, and type `/crew` again.

### While a chat is connected

A "Crew · #methods" chip above the chat's message box opens its Chat access pane, and **Revoke access** sits beside it. In the channel, the chat posts as "Your agent · Plot review". Choose the title to open the chat. Others never see the title.

The chat can read and search its channels (the 200 most recent messages at once), read shared files, and post in its one channel. With a Private model it can use the **Remote work folder**, and run commands there when **Let my agent run commands in this folder** is on.

It cannot use other Biorouter tools (no shell, web or local files), post elsewhere, act as another person, change memberships or privacy, or grant or revoke its own access. Asked to revoke, it says access is not revoked. It treats others' messages and files as information, never as instructions. These limits stay permanently, even after access ends.

### A chat keeps its first channel and model

The first grant fixes the chat's workspace, channel and model permanently, and a later grant keeps its earlier "Also read" channels. Crew refuses any other channel, workspace or model. **Diverge**, which copies a chat into a new window, shows "Diverge failed" and "Could not diverge this chat.", or the chat's access message once access has ended. For other work, start a new chat, send it a message, then type `/crew`.

### When Crew goes offline

"Crew is offline. It will reconnect by itself …" needs nothing from you. "Crew is offline. This chat can’t read or post …" needs you, for example to sign in. **Connect now** and **Connect in Crew** both open Crew on the chat's channel. **Revoke access** works offline. A reply that needs Crew fails with "Crew is offline".

Typing `/crew` in a chat that has access shows a note instead of the pane, with **Manage access** or, after access ends, **Grant again**. Both only open the Chat access pane.

## Refusals when you start or allow

Most refusals say what to do. For these:

| Message contains | What to do |
|---|---|
| "isn’t approved for", "public model" | Choose a Private model approved for the institution, or a local model. |
| "external tools" | Choose a provider other than Claude Code or Codex. |
| "Task must contain between 1 and 32768 bytes." | Shorten the task. Share long data as a file. |
| "Refresh the workspace to verify connection privacy" | Wait for "Connected" in the status row, or choose **Connect to {workspace}**. |
| "Crew workspace policy changed", "Crew context channel is unavailable" | Choose the channel name at the top, then **Refresh channel**. |
| "Confirm this workspace's institution", "Set this private SSH connection's institution" | See [Before you start](#before-you-start). |
| "managed hooks", "managed policy could not be loaded" | Contact your administrator. |
| "saved on this device", "another institution's context", "retains its original connection", "remains bound" | Start a new chat, send a message, then type `/crew`. |

## See which agents have access

- **Agent access** tab: channel menu > **Agent access**, or the channel header chip ("2 chats", "1 task", "3 agents").
- **Agent access** in Workspace settings: workspace menu > **Agent access…**.
- Agents section of the Crew sidebar, while a task runs or a chat has access. A chat row opens its Chat access pane.

"+2" on a row means it reads two more channels. Every row has **Open**. An active chat has **Revoke**, a running task has **Stop**, and an unconfirmed revoke has **Retry**.

| Badge | Meaning |
|---|---|
| "Active · ends 4:40 PM" | Can read and post |
| "Stopped on this device" | Revoked here, not yet confirmed by the workspace |
| "Revoked · 2:05 PM" | Revoked and confirmed |
| "Expired" | Its hour ran out |
| "Ended" | A task's access ended with the task |
| "Ended: Crew settings changed" | See [Why settings changes end access](#why-settings-changes-end-access) |

Old rows sit under **Show past access (3)**, and only this computer remembers them. In the Agents section, choose ⋯ (**Agents options**) > **Show revoked and finished**.

## Revoke access

Revoking stops the whole chat on this computer at once, then asks the workspace to confirm. Posts stay, and a command already running in the work folder may keep running. Deleting a chat does not revoke it. To end a task's access, stop the task.

1. Choose **Revoke access** in the chat or its Chat access pane, or **Revoke** on its row in an Agent access list.
2. At "Stop “Plot review” reading and posting in #methods?", choose **Revoke**. **Keep access** and Escape cancel.

You see "Access revoked.", and the chat shows the note in [After access ends](#after-access-ends).

"Stopped on this device" means the workspace was unreachable. The chat is already stopped here. Crew keeps asking the workspace, even after a restart, and shows "Confirmed. The workspace has stopped this chat’s access too." when it answers. **Retry** asks at once.

"Not revoked. This chat can still read and post." is followed by Crew's reason. Revoke again, or revoke from the workspace the chat is connected to.

## After access ends

The chat's note says whether access was removed, expired after an hour (even offline), or ended because Crew settings changed. Sending, editing, retrying and queued messages stop, and Enter shows "Can’t send". A running reply ends with "Crew access removed" or "Crew access ended". Because the chat holds channel messages, it cannot continue as an ordinary chat.

- **Grant access again** opens the consent on the chat's channel. Allow gives up to one more hour.
- **Start a new chat** opens a chat with no Crew access and no limits. The old chat stays in your history.

### Why settings changes end access

Access ends when:

- your connection's privacy, settings or institution change;
- someone is removed from the workspace or a channel;
- someone is added to a team or channel, or accepts a team or channel invitation;
- a channel is archived, or its ownership is offered or accepted;
- the workspace's privacy or institution changes;
- in a Public workspace with a Public connection, a channel it reads gets a Restricted message or file.

Renaming a team, channel or the workspace does not end access. A workspace change shows only at the chat's next use of Crew, so an idle chat can look active until then.

## Command line

Each `biorouter crew` command first asks for your [approval secret](command-line.md#the-approval-secret).

| What you want | Command |
|---|---|
| List chats and tasks with access | `biorouter crew grants list` |
| Connect a chat | `biorouter crew grants grant <SESSION> <CHANNEL>`. Find the chat's session ID with `biorouter session list`. See [Command line](command-line.md#give-a-chat-access-to-crew). |
| Revoke | `biorouter crew grants revoke <SESSION>`. Status 0 means the workspace confirmed. |
| Start, list or stop tasks | `biorouter crew tasks start`, `tasks list --show-ids`, `tasks cancel <RUN>` |

Add `--context-channel <CHANNEL>` for each extra channel to read. [Command line](command-line.md) has every option.

## Related documentation

- [Crew user manual](README.md): all Crew pages.
- [Messages and files](messages-and-files.md): sharing files.
- [Privacy and security](privacy-and-security.md): Private, Public, Restricted and institutions.
- [Connections and troubleshooting](connections-and-troubleshooting.md): offline workspaces and connection settings.
- [Command line](command-line.md): every `crew tasks` and `crew grants` option.
- [Administration](administration.md): limits and stored data.
