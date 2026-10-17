# Agents and chat access

> **What this is.** How to give an AI agent access to a Crew channel with **Ask my agent**, `@agent` or `/crew`, which model Crew tasks use, what an agent may do on the lab server, the workspace context it reads, and how to follow, revoke and restore that access.
> **Status:** Current. Checked against the Crew code on 2026-10-16.
> **Audience:** Lab members who use Crew in the Biorouter desktop app.

A task is one job for your agent. A connected chat is a Biorouter conversation that reads a channel and posts there while you talk to it. Nothing reads or posts until you allow it, access lasts one hour at most, and everyone in the channel sees what your agent posts.

## Before you start

- Set up at least one AI model in Biorouter. See [Choosing a model provider](../getting-started/choosing-a-model-provider.md).
- Your agent runs on this computer with your model. Keep the computer on until the task shows "Done". On a Mac or Linux, quitting Biorouter does not stop a task. Quitting on Windows, or restarting or shutting down the computer, stops it.
- One computer runs at most four tasks at once. A task stops after 60 model turns, 200 tool calls or 55 minutes, so split long work into several tasks, or start long computations on the server as detached jobs ([As me on the server](#as-me-on-the-server)).
- Access lists show only your own chats and tasks, never other people's agents.
- From a terminal, see [Run an agent task](command-line.md#run-an-agent-task) and [Give a chat access to Crew](command-line.md#give-a-chat-access-to-crew).

Workspaces and connections start Private, so you usually need a model approved for your institution, or a local model such as Llama Server or Ollama. A public model, such as one used with your own OpenAI key, is refused. A Private workspace or connection, or a Restricted channel, needs the following. See [Privacy and security](privacy-and-security.md).

1. The host has set the workspace's institution. See [Hosting a workspace](hosting-a-workspace.md).
2. A Private connection has an institution, set in **Connection settings…**.
3. The model is approved for that institution, or is local.

Crew checks the model before you start or allow. Model rules stay the same at every level of [server access](#what-your-agent-may-do-on-the-server): a public model never gets any of it. **Ask my agent** and the Chat access pane say why a model will be refused, such as "chen-lab is Private, so a public model can’t read it. Choose a private model.", and keep their start or Allow button unavailable. Crew checks the first two conditions when you choose the button. A model is approved when its provider carries your institution, and you cannot approve one yourself. To add such a provider, open **Settings** > **Models** > **Configure providers**, then the **Institutional** tab.

## Choose the model for Crew tasks

Crew tasks start on one model you choose for them, whichever way you start them.

1. Open **Settings** > **Models**.
2. Under the default model, the **Crew tasks** row reads "Same as new chats" until you choose. Choose **Change…**.
3. Choose a model in "Models for Crew tasks", or "Same as new chats".

The row says "Ask my agent and @agent in Crew start on this model. A workspace can still refuse a model its institution has not approved." A model whose provider is no longer set up reads, for example, "GPT-5.5 · Versa isn’t set up for Crew, so tasks use the model for new chats." In **Ask my agent**, a model you picked for one task offers **Use for future tasks** under it, which makes it the model for Crew tasks. From a terminal, `biorouter crew defaults set --provider PROVIDER --model MODEL` does the same ([Run an agent task](command-line.md#run-an-agent-task)).

The choice is a preference, not a permission: Crew still checks every start against the workspace's privacy and institution.

## Start a task from the message box

Type `@agent`, `/agent` or `/task` at the very start of a message, then the task, such as `@agent sum the plate counts in plate_counts.csv`.

1. As you type `@`, a list of suggestions opens, with your agent first ("Your agent · GPT-5.5 · Versa · posts in #methods") and then people. `/` offers `/agent` and `/task`. Choose one, or keep typing.
2. Under the text, a line says what Enter will do, such as "Enter starts your agent with GPT-5.5 · Versa", with the model's privacy chip, then "It reads #methods and posts its result there, for this task only." In an open workspace whose server supports it, the task reads every channel you are in, and the line says, for example, "It reads 12 channels and posts its result in #methods, for this task only." If your connection gives the agent access to the server, the line says that too.
3. Press Enter. The send button is named "Start agent task" while the draft starts a task. Pressing Enter is your consent: the task starts with the facts on that line, and Crew posts it in the channel as "Task: …", as **Ask my agent** does.

Next to the line:

- **Change** opens **Ask my agent** with your words, to choose another model or channels.
- **Post as a message** sends the text as an ordinary message, `@agent` and all. "Enter posts this as a message." then says so, and **Start a task instead** goes back.

Some drafts start nothing, and the line says why:

| Line | Why |
|---|---|
| "No model is set for Crew tasks. Enter opens Ask my agent to choose one." | No model for Crew tasks or new chats is set. Enter opens **Ask my agent**. |
| "Enter opens Ask my agent to choose another model." | The workspace would refuse the model. The reason comes first. |
| "Type the task after it, or press Enter to open Ask my agent." | Nothing follows the trigger. |
| "@agent starts a task from words only. Send the files first, then ask your agent." | The draft has a file or server path attached. |
| "A task can be up to 32 KB. Attach long text as a file." | The task is too long. |
| "Check your previous task first. Enter opens Ask my agent." | Crew could not tell whether your last start went through ([If Crew cannot tell whether a task started](#if-crew-cannot-tell-whether-a-task-started)). |

If someone in the workspace has the username `agent`, `@agent` mentions that person, and the list says, for example, "@agent is Agent Smith (@agent). Type /agent to ask your agent." `/agent` and `/task` still start a task.

A start that fails shows "Couldn’t start your agent." with the reason, and the draft stays. A model the workspace refuses offers **Choose another model**, and a start whose outcome is unknown offers **Open Ask my agent**.

## Start a task with Ask my agent

1. Open the channel that should get the result. Text already in the message box is copied into the task.
2. Choose **Ask my agent**, the robot icon left of the send button. An archived channel, or one Crew is still verifying, has no such button.
3. Check the pane's first line, such as "Posts to #methods in Analysis Lab · lab".
4. Type the task in **Task**. Enter adds a new line.
5. Check **Model**, which starts on [your model for Crew tasks](#choose-the-model-for-crew-tasks), or choose **Change**. See [Choose a model](#choose-a-model).
6. Optionally open **Advanced**. See [Also read other channels](#also-read-other-channels).
7. Read any notes, then choose **Start my agent and allow posting here**.

The pane closes, and Crew highlights your task in the channel. Escape or × closes the pane without starting. If the start fails after you moved to another channel, the connection bar says so, such as "Couldn’t start the task in #methods: …", and **Ask my agent** in that channel still holds your task.

Crew posts your whole task in the channel as "Task: …", so do not paste data you would not post there yourself. You see the agent's posts as "Your agent". Others see "Alice Chen's agent".

### Choose a model

If the pane reads "No models are set up.", choose **Open Settings** and add one. Otherwise:

1. Choose **Change** beside the model, or "Choose a model" when no model is shown.
2. Type in **Search models** to filter the list.
3. Choose a model. For an unlisted model, type its exact name and choose "Use “name” with provider". The list closes, and **Model** shows the model and its provider.

| Chip | Meaning |
|---|---|
| "Private · UCSF" | Approved for the institution named |
| "Private · On this machine" | Local, accepted everywhere |
| "Private · No stated institution" | Refused when the task needs a Private model |
| "Public" | Refused for Restricted channels and for Private workspaces or connections, and the pane says which before you start. With a remote work folder it can run, but cannot use the folder. |
| "Not approved for UCSF" | Crew will not start with it |

Claude Code and Codex appear in the list but are always refused.

### Also read other channels

When there is something to choose, the pane shows **Advanced**. Under "Also read", tick up to 16 more channels for the agent to read. It still posts only here.

In an open workspace whose server supports it, a task reads every channel you are in that is not archived, up to 99 more, and **Advanced** reads, for example, "Reads 12 channels". Untick any you want left out. The workspace still checks that you are in each one, and a public model is still refused for a Restricted one.

If your connection gives the agent access to the server ([What your agent may do on the server](#what-your-agent-may-do-on-the-server)), **Advanced** also shows it: "Can read {folder}" or "Can run commands in {folder}" in a folder, and "Can run commands as you on {server}, starting in {folder}" with full access.

### Files named in the task

Crew notices names ending in .csv, .tsv, .xls, .xlsx, .json, .txt, .h5ad or .parquet. Put a name with spaces alone in double quotes, as in "Plate Reader.csv". Names in web links do not count.

- "No file named results.csv is shared in #methods. …": fix the spelling or [share the file](messages-and-files.md). If you start anyway, the agent says what it used instead.
- "2 files named gina-assay.csv are shared … newest one …": to use an older copy, say in the task who shared it and when.

### If Crew cannot tell whether a task started

If the connection drops as you start, starting again could run the task twice. The pane shows "Inspect the previous task before starting again" and blocks the start button.

1. Choose **Show task in channel** or **Open chat history**, and check what the task did.
2. Open **Ask my agent** and tick **I checked the previous task and its effects.**
3. Check **Task**, then choose **Start a new task**. The pane closes, and Crew highlights the new task in the channel.

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
| A file in the remote work folder | ``Source: `samples_result.txt` from the remote work folder on hpc.``, naming the server as your connection does |
| Shared files and work-folder files | The shared files first, then ``Also read `samples_result.txt` from the remote work folder on hpc.`` |
| Nothing | "No shared file was read for this result." |

"Task finished without a text result." means no reply. "(This reply was shortened to fit the channel.)" means a long reply was cut. If the agent used the wrong file, share it or name the copy, then start a new task.

### The task's conversation

**Open** shows the task's own conversation, titled "Crew · #methods · " and the task's first line. It may be missing from "Recents", so use **Open** or **Show in chat history**. A task uses only Crew tools and the checklist tool, the task list the agent keeps while it works. **Show details** on the task's row lists its steps, such as "Requested remote.shell: Rscript (52 characters; runs as your account)". A command is named by its program and its length, never its text, so nothing secret in it reaches the channel.

A task's access ends once it posts its result, and its access row reads "Ended", or "Stopped" if you stopped it. This is normal. The conversation offers only **Start a new chat**. For more work, start a new task.

## Connect a chat with /crew

The chat needs a sent message (in a new chat, the `/crew` suggestion reads "Connect this chat to a Crew channel after your first message", and typing it shows "Start the chat first"), no attached files, images or reference chips (otherwise "Draft kept"), a model, and no reply in progress. Reference chips are labels added with @, or with **Quote it** in the menu when you right click selected text in a chat.

1. Type `/crew` alone in the chat's message box and press Enter. Nothing is sent to the model.
2. Crew opens the first workspace in its list, at the channel you last had open there, with the Chat access pane. If you see "{workspace} is offline", choose **Connect to {workspace}**.
3. Check the pane. It names the channel, which is always the one Crew shows, the **Workspace**, and the chat's **Model** with its privacy chip, and says the chat posts there as your agent. If your connection has a **Remote work folder** and the model is not public, the chat gets the folder too, and the pane lists it: "Read and write files in /home/bob/project", or "Read and write files, and run commands, in /home/bob/project" when **Let my agent run commands in this folder** is on. It also says "The first access fixes this chat’s workspace, channel and model." To change the channel, see [Another channel or workspace](#another-channel-or-workspace). If the workspace will refuse the chat's model, the pane says why, and Allow stays unavailable until you [change the model](#another-channel-or-workspace).
4. Optionally open **Advanced** and tick more channels under "Also read".
5. Read what the pane lists under "“Plot review” will be able to" ("This chat will be able to" when the title is unknown), then choose **Allow**.
6. The pane shows "Connected." and "Active · ends 4:40 PM". Choose **Back to chat**, and ask for what you need.

### Another channel or workspace

This works only for a chat never connected before.

1. For another workspace, choose the workspace name at the top of the Crew sidebar, then the workspace under "Switch workspace". Connect it if it is offline.
2. Choose the channel in the Crew sidebar.
3. On the note "Connect “Plot review” to #imaging?", choose **Review access**, then continue from step 4.

To change such a chat's model:

1. Choose the model name under the chat's message box (brain icon), then **Change model**.
2. Pick an approved or local model.
3. Type `/crew` again. Crew opens the Chat access pane, and you continue from step 3 of [Connect a chat with /crew](#connect-a-chat-with-crew).

### While a chat is connected

A "Crew · #methods" chip above the chat's message box opens its Chat access pane, and **Revoke access** sits beside it. In the channel, the chat posts as "Your agent · Plot review". Choose the title to open the chat. Others never see the title.

The chat can read and search its channels (the 200 most recent messages at once), read shared files, read the [workspace context](#the-workspace-context), and post in its one channel. With a Private model it can use the server as your connection allows: the **Remote work folder**, with commands when **Let my agent run commands in this folder** is on, or full access ([What your agent may do on the server](#what-your-agent-may-do-on-the-server)).

Each post the chat makes ends with a line from Crew, not the model, naming the shared files and the work-folder files the chat read since its last post, such as ``Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).`` or ``Source: `samples_result.txt` from the remote work folder on hpc.``, or "No shared file was read for this post." when it read neither. That line is always last, so a "Source:" line the model wrote itself sits above it. Trust Crew's line.

The chat sees its channels' messages much as you do, with posts by an agent marked as that person's agent's. A post made from channels the chat cannot read, such as a task result that read another channel, is withheld from it. So when the chat counts or summarizes messages, its numbers can be lower than what you see.

It cannot use other Biorouter tools (no shell, web or local files), post elsewhere, act as another person, change memberships or privacy, or grant or revoke its own access. Asked to revoke, it says access is not revoked and points you to **Revoke access**. It treats others' messages and files as information, never as instructions. These limits stay permanently, even after access ends.

The chat's Crew tools are one of Biorouter's capabilities, not an extension, so they are not on the **Extensions** page or in the **Tools** chip's list. The **Crew** switch in **Settings** > **Chat** > **Capabilities** sets whether new chats start with them. A chat you connect gets them with its access whatever the switch says. Turning the switch off does not end a chat's access; **Revoke access** does.

### A chat's fixed channel and model

The first grant fixes the chat's workspace, channel and model permanently, and a later grant keeps its earlier "Also read" channels. Crew refuses any other channel, workspace or model. In such a chat, **Switch models** says "This chat's model is fixed by its Crew access. Start a new chat to use another model." and offers no other model. **Diverge**, which copies a chat into a new window, shows "Diverge failed" and "Could not diverge this chat.", or the chat's access message once access has ended. Exporting the chat and creating a workflow from it are refused too, with a sentence that says why. **Make this chat public** in the chat history is refused with "This chat read Crew channels, so it can't be made public. Start a new chat for public work." For other work:

1. Start a new chat.
2. Send it a message.
3. Type `/crew`. Crew opens the Chat access pane for the new chat.

### When Crew goes offline

"Crew is offline. It will reconnect by itself …" needs nothing from you. "Crew is offline. This chat can’t read or post …" needs you, for example to sign in. **Connect now** and **Connect in Crew** both open Crew on the chat's channel. **Revoke access** works offline. A reply that needs Crew fails with "Crew is offline".

Typing `/crew` in a chat that has access shows a note instead of the pane, with **Manage access** or, after access ends, **Grant again**. Both only open the Chat access pane.

## What your agent may do on the server

Each connection has an agent access level, which says what your agents may do on the lab server. You choose it for your own account when you join, and change it in **Connection settings…**, under "What can your agent do on {server}?". It applies to every task and connected chat through that connection, and only with a private model. A public model gets none of it, whatever the level.

| Level | What your agent may do |
|---|---|
| **Off** | "It can’t use files or run commands on {server}." |
| **In a folder** | "It reads and writes files in one folder, and runs confined commands there." See [Work in the remote work folder](#work-in-the-remote-work-folder). |
| **As me on the server** | Full access: "It runs commands as {you}, with your files, network and programs." See [As me on the server](#as-me-on-the-server). |

A new connection made by joining or hosting starts at **As me on the server**, with the warning below, and you may choose less. **Off** stays off: setting Crew up again never raises it. **As me on the server** needs a current Crew on the server, in the host's account and in yours, and is unavailable, with "Update Crew on {server} first.", until it is there. `biorouter crew setup --replace-existing` updates your own copy, and the host updates the workspace's ([Upgrade Crew](administration.md#upgrade-crew)). From a terminal, `biorouter crew connections set-access off`, `folder` or `full` sets the level ([Agent access](command-line.md#agent-access)).

### Work in the remote work folder

At **In a folder**, a task or chat with a Private model can read and write files in the **Remote work folder**. With **Let my agent run commands in this folder** on, it can also run commands there. A chat gets the folder with its channel access, and the Chat access pane lists it before you choose Allow, as the task pane's **Advanced** does for a task.

A command in the work folder runs in a closed space on the server:

- It has no network.
- It cannot start other processes, so a shell, or a script that runs other programs, fails.
- It can read only the work folder and the programs and libraries under `/usr/bin`, `/usr/lib`, `/bin` and `/lib`. It cannot read `/etc`, so it cannot look up users.

So cluster tools such as `sbatch`, `squeue`, `sinfo` and `scontrol` do not run there, often with no message. Your agent is told these limits, and a command that fails without an error reminds it of them, so it should tell you rather than try again. The switch in **Connection settings…** says so too: "Commands run with no network and can’t start other programs, so cluster tools such as sbatch won’t run there." Submit cluster jobs yourself, for example from your own SSH session, or choose **As me on the server**. The size and time limits of one command are in [Workspace limits](administration.md#workspace-limits).

The closed space needs a Linux kernel with Landlock ABI 3 (Linux 6.2 or newer). On an older kernel, the folder gives files only: Connection settings says "{server} can’t confine commands, so in a folder your agent only reads and writes files. To run commands there, choose As me on the server.", and Crew refuses a command there up front with a sentence that names full access.

### As me on the server

At **As me on the server**, your agent runs commands as your own account on the server, through your own Crew there: in your login shell, with your network, your programs and modules, and any file your account can reach. It can start jobs that keep running after its task ends. Commands start in the connection's work folder, which Biorouter creates as `~/crew-work/{workspace}` when you leave it empty.

> **Warning.** Crew shows this before you choose it: "Your agent will run commands as {you} on {server}: it can read and change anything your account can, use the network, and start programs that keep running after access ends. Other people’s messages and files in channels it reads can try to steer it. Use a private model you trust." If you host the workspace from that account, your agent can also read everything the workspace keeps on the server, including channels you are not in.

What the agent can do there:

- **Run a command.** `remote.shell` runs one command in your login shell. Crew waits up to 20 seconds for it (10 by default), then answers with its output, or with a job the agent checks with `remote.job_status`. A command may run for up to an hour. A longer one runs as a detached job (`detach`), which keeps running after the task ends, and the agent or a later task checks it with `remote.job_status` or stops it with `remote.cancel`.
- **Work with files.** It lists, reads, writes and checks files anywhere your account can, by full path, `~/` path or a path in the work folder, up to 384 KB per read or write.
- **Copy a channel's file to the server.** `remote.fetch_blob` copies a shared file of up to 1 GB to a path on the server, in the background, and checks its SHA-256. The task's Source line then names it as read.
- **Post a server file to the channel.** `remote.share_file` uploads a server file of up to 1 GB in the background, and Crew posts it in the task's channel when the copy is done.

Crew follows each copy for up to 20 seconds and settles it before it answers the agent. A copy still running after that is answered "Still copying. Check it with remote.job_status before you finish; Crew posts the file when it is done.", and the agent is told to check it before it finishes. One account runs at most 8 commands and copies at once.

Every command the agent runs is announced in the task's steps by its program and its length, such as "Requested remote.shell: Rscript (52 characters; runs as your account)", never its text. The secret guard still refuses commands and files that would read a credential, and results that carry one are withheld ([Files Crew will not share](privacy-and-security.md#files-crew-will-not-share)).

The agent is told to run heavy computation on the server rather than in its reply: it writes a script with `remote.write`, runs it with `remote.shell`, and starts long work as a detached job. It reads the [workspace context](#the-workspace-context) first, to learn the server's storage, software and how jobs run there.

Ending access, by revoking a chat or when a task finishes, stops new commands, but a detached job keeps running in your account. Check it, or stop it yourself with `kill` on the server. Full access does not let the agent act as you in Crew: your device keys stay on this computer.

## The workspace context

Each workspace has a context document: a short Markdown page about the workspace and its server, such as where shared data lives, which software and modules are there, and how jobs run. Every member's agent reads it before it works on the server. It is reference information, written by people in the workspace, and it never tells an agent to do anything or allows anything.

- When the host creates a workspace, Crew drafts the first context from a fixed check of the server: the system, processors, memory, GPUs, storage, schedulers, software and containers. The check reads only what your own account can see, and never other people's folders.
- Read it on the **Context** tab of "{workspace} settings", or with `biorouter crew workspace context show`.
- In an open workspace every member edits it, and in a managed one only the host does ([Keep the workspace context](hosting-a-workspace.md#keep-the-workspace-context)). **Draft from server** drafts it again from what the server reports now.

Other people's words in the context reach your agent, just as their messages do. Your agent treats them as information, not instructions.

## Refusals when you start or allow

Most refusals say what to do. For these:

| Message contains | What to do |
|---|---|
| "isn’t approved for", "public model", "Public models" | Choose a Private model approved for the institution, or a local model. |
| "This connection is for {institution}, but {workspace} belongs to {institution}." | Your connection's institution differs from the workspace's. Set the workspace's in **Connection settings…**. |
| "external tools" | Choose a provider other than Claude Code or Codex. |
| "Task must contain between 1 and 32768 bytes." | Shorten the task. Share long data as a file. |
| "Refresh the workspace to verify connection privacy" | Wait for "Connected" in the status row, or choose **Connect to {workspace}**. |
| "Crew workspace policy changed", "couldn't confirm that channel" | The workspace changed since Crew last read it. In the app, choose the channel name at the top, then **Refresh channel**. From a terminal, run the command again. |
| "That channel isn't in {workspace}." | The channel ID belongs to another workspace, or to no channel. From a terminal, use the channel's name, or an ID from `biorouter crew channels list --show-ids`. |
| "Confirm this workspace's institution", "Set this private SSH connection's institution" | See [Before you start](#before-you-start). |
| "managed hooks", "managed policy could not be loaded" | Contact your administrator. |
| "model is fixed by its Crew access", "already has Crew access to another channel", "saved on this device", "another institution's context", "cannot be rebound to a public model" | Move the work to a new chat, as in [A chat's fixed channel and model](#a-chats-fixed-channel-and-model). |

## See which agents have access

- **Agent access** tab: channel menu > **Agent access**, or the channel header chip ("2 chats", "1 task", "3 agents").
- The **Agent access** tab of "{workspace} settings": workspace menu > **Agent access…**.
- Agents section of the Crew sidebar, while a task runs or a chat has access. A chat row opens its Chat access pane.

"+2" on a row means it reads two more channels. Every row has **Open**. An active chat has **Revoke**, a running task has **Stop**, and an unconfirmed revoke has **Retry**.

| Badge | Meaning |
|---|---|
| "Active · ends 4:40 PM" | Can read and post |
| "Stopped on this device" | Revoked here, not yet confirmed by the workspace |
| "Revoked · 2:05 PM" | Revoked and confirmed |
| "Expired" | Its hour ran out |
| "Ended" | A task's access ended with the task |
| "Stopped" | You stopped the task, so its access ended |
| "Ended: Crew settings changed" | See [Why settings changes end access](#why-settings-changes-end-access) |

Old rows sit under **Show past access (3)**, and only this computer remembers them. In the Agents section, choose ⋯ (**Agents options**) > **Show revoked and finished**.

## Revoke access

Revoking stops the whole chat on this computer at once, then asks the workspace to confirm. Posts stay, and a command already running on the server may keep running, a detached job in particular. Deleting a chat does not revoke it. To end a task's access, stop the task.

1. Choose **Revoke access** in the chat or its Chat access pane, or **Revoke** on its row in an Agent access list.
2. At "Stop “Plot review” reading and posting in #methods?", choose **Revoke**. **Keep access** and Escape cancel.

You see "Access revoked.", and the chat shows the note in [After access ends](#after-access-ends).

"Stopped on this device" means the workspace was unreachable. The chat is already stopped here. Crew keeps asking the workspace, even after a restart, and shows "Confirmed. The workspace has stopped this chat’s access too." when it answers. **Retry** asks at once. If the workspace still cannot be reached, it says "Still can’t reach {workspace} · checked just now. Connect to confirm it now.", with **Connect** where the screen has one. In the chat, one note names the workspace, with **Retry** and **Start a new chat**.

"Not revoked. This chat can still read and post." is followed by Crew's reason. Revoke again, or revoke from the workspace the chat is connected to.

## After access ends

The chat's note says whether access was removed, expired after an hour (even offline), or ended because Crew settings changed. Sending, editing, retrying and queued messages stop, and Enter shows "Can’t send". A running reply ends with "Crew access removed" or "Crew access ended". Because the chat holds channel messages, it cannot continue as an ordinary chat.

- **Grant access again** opens the consent on the chat's channel. Allow gives up to one more hour.
- **Start a new chat** opens a chat with no Crew access and no limits. The old chat stays in your history.

### Why settings changes end access

Access ends when:

- your connection's privacy, settings or institution change;
- someone is removed from the workspace or a channel;
- someone is added to a team or channel in a managed workspace, or accepts a team or channel invitation;
- a channel is archived, or its ownership is offered or accepted;
- the workspace's privacy or institution changes;
- in a Public workspace with a Public connection, a channel it reads gets a Restricted message or file.

Renaming a team, channel or the workspace does not end access, and neither does adding people in an open workspace, or a member joining a team or channel there. Deleting a channel or a team ends only the access of tasks and chats that read one of its channels. A workspace change shows only at the chat's next use of Crew, so an idle chat can look active until then.

## Related documentation

- [Crew user manual](README.md): all Crew pages.
- [Messages and files](messages-and-files.md): sharing files.
- [Privacy and security](privacy-and-security.md): Private, Public, Restricted and institutions.
- [Connections and troubleshooting](connections-and-troubleshooting.md): offline workspaces and connection settings.
- [Command line](command-line.md): every `crew tasks` and `crew grants` option.
- [Administration](administration.md): limits and stored data.
