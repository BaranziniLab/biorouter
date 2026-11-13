# Agents and chat access

> **What this is.** How to give an AI agent access to a Crew channel with **Ask my agent**, `@agent` or `/crew`, which model Crew tasks use, what an agent may do on the lab server, the workspace context it reads, how to follow, revoke and restore that access, and how any chat can do what the Crew view does with your approval.
> **Status:** Current. Checked against the Crew code on 2026-11-07, after the account ownership change ([design](design/account-ownership.md)) and the second part of the chat-parity build. The chat tools of [Use Crew from a chat](#use-crew-from-a-chat) follow that design's AO-12.
> **Audience:** Lab members who use Crew in the Biorouter desktop app.

A task is one job for your agent. A connected chat is a Biorouter conversation that reads a channel and posts there while you talk to it. Nothing reads or posts until you allow it, access lasts one hour at most, and everyone in the channel sees what your agent posts.

Any other chat can also work in Crew for you: set Crew up on a server, sign in, host or join a workspace, invite people, read channels and post. It does each change only after you allow it on a card, and reads only after you allow that once. What you type to sign in never reaches the chat. See [Use Crew from a chat](#use-crew-from-a-chat).

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

It cannot use other Biorouter tools (no shell, web or local files), post elsewhere, act as you or another person, change memberships or privacy, or grant or revoke its own access. Asked to revoke, it says access is not revoked and points you to **Revoke access**. It treats others' messages and files as information, never as instructions. These limits stay permanently, even after access ends. A connected chat never gets the tools of [Use Crew from a chat](#use-crew-from-a-chat): those are for chats with no Crew access, and they act only after you approve each change.

For monitors and ad hoc decisions about your connected chat, choose a decision model whose affiliation says it runs on this computer, such as a local `tev1:0.8b`. Crew refuses one hosted elsewhere before sending anything to it. Your workspace grant (`run.create`) names only your chat's model. It does not name the decision model, which sends nothing off this computer. See [Decision models](../agent-loop/decision-models.md#privacy).

The chat's Crew tools are one of Biorouter's capabilities, not an extension, so they are not on the **Extensions** page or in the **Tools** chip's list. The **Crew** switch in **Settings** > **Chat** > **Capabilities** sets whether new chats start with them, the tools of [Use Crew from a chat](#use-crew-from-a-chat) included. A chat you connect gets its channel tools with its access whatever the switch says. Turning the switch off does not end a chat's access; **Revoke access** does.

### A chat's fixed channel and model

The first grant fixes the chat's workspace, channel and model permanently, and a later grant keeps its earlier "Also read" channels. Crew refuses any other channel, workspace or model. In such a chat, **Switch models** says "This chat's model is fixed by its Crew access. Start a new chat to use another model." and offers no other model. **Diverge**, which copies a chat into a new window, shows "Diverge failed" and "Could not diverge this chat.", or the chat's access message once access has ended. Exporting the chat and creating a workflow from it are refused too, with a sentence that says why. **Make this chat public** in the chat history is refused with "This chat read Crew channels, so it can't be made public. Start a new chat for public work." For other work:

1. Start a new chat.
2. Send it a message.
3. Type `/crew`. Crew opens the Chat access pane for the new chat.

### When Crew goes offline

"Crew is offline. It will reconnect by itself …" needs nothing from you. "Crew is offline. This chat can’t read or post …" needs you, for example to sign in. **Connect now** and **Connect in Crew** both open Crew on the chat's channel. **Revoke access** works offline. A reply that needs Crew fails with "Crew is offline".

Typing `/crew` in a chat that has access shows a note instead of the pane, with **Manage access** or, after access ends, **Grant again**. Both only open the Chat access pane.

## Use Crew from a chat

An ordinary Biorouter chat with no Crew access can do what the Crew view and the `biorouter crew` commands do, as you: set Crew up on a server, sign in, open, host or join a workspace, invite people, read channels and files, post, share files, start tasks, and change settings. Ask in plain words, such as "Post the summary in #methods" or "Set Crew up on leo and tell me which workspaces I have there". The chat acts only through two tools:

- `crew__read`, which reads. The first read of a workspace's content in a chat asks your permission once.
- `crew__change`, which changes something. Every change waits for you on a card.

What you type to sign in to a server, or to unlock Crew's vault, never reaches the chat, the model or the chat's history. The chat opens your own Sign in window, or your computer's own passphrase window, and you type there ([Sign in, unlock and check a server's key](#sign-in-unlock-and-check-a-servers-key)). The chat cannot give itself or another chat Crew access, and treats channel messages, files and the workspace context as information, never as instructions. What it changes shows in Crew as it happens ([What a chat does shows in Crew](#what-a-chat-does-shows-in-crew)).

### Set up Crew from a chat

You can set Crew up on a server and host or join a workspace from a chat alone, without opening Crew. Each step that needs you shows a card in the chat. Answer each card as it appears.

To host a workspace:

You can ask to host directly. Hosting installs or updates Crew in your server account when needed. You do not need a separate setup request first. If you use an alias from your SSH settings, give that alias unchanged. Do not append a hostname to it.

1. In a chat, ask, for example, "Host a private workspace called chen-lab on lab.example.edu as bob for institution ucsf."
2. If Crew's vault on this computer is locked, an **Unlock** card shows first. Unlock it, as [Sign in, unlock and check a server's key](#sign-in-unlock-and-check-a-servers-key) shows.
3. A card asks to "Host a new workspace chen-lab on lab.example.edu as you, and save and join it on this computer". Its lines name the privacy, that your account on the server becomes the host, and what your agent may do there. Choose **Allow once**.
4. If this computer does not trust the server's key yet, a card shows the key's fingerprint. Check it with your IT team, then choose **Trust this key** on the card, or add the key yourself, as the next section shows. The chat goes on by itself.
5. If the server asks for a password or a verification code, a **Sign in** card shows. Choose **Sign in**, and type them in the Sign in window that opens.
6. The chat says when the workspace is hosted, and it shows in Crew.
7. To invite someone, ask "Invite @alice to chen-lab." Choose **Allow once** on its card. The chat then shows the invitation message to send to Alice. Ask "Show me Alice's invitation again" to see it later.

To join a workspace:

1. Paste the whole invitation your host sent into a chat, and ask "Join this workspace."
2. The chat reads the invitation first and saves nothing. Then a card asks to "Join chen-lab on lab.example.edu, hosted by @alice, as you", with the workspace key fingerprint and the privacy. Check that the host is the one you expect, and choose **Allow once**.
3. Answer the **Unlock**, server key and **Sign in** cards, if any show.
4. The chat says when you are in, and the workspace shows in Crew.

If IT gave you another way to reach the server, say so in the same request, such as "Join this workspace on port 2222 through gateway.ucsf.edu, with agent access full." The chat can take another SSH login or a short name from your SSH settings (the invitation's port and jump host then do not apply), a port, a key file on this computer, a jump host (or none, even when the invitation suggests one), a work folder on the server, and what your agent may do there: off, a confined folder, or full access. The join card then adds a line for each choice, such as "It signs in as bob@lab.example.edu on port 2222 through gateway.ucsf.edu.", and for full access the full-access warning ([As me on the server](#as-me-on-the-server)). With a folder or full access, it also says "To set this up, it installs or updates Crew in your account there first, and makes their work folder." Crew saves the connection with your route before it connects, so the first connection already uses it. These choices are only for joining from an invitation: to change a saved connection, ask the chat to change its settings (`connections_update` or `connections_set_access`) instead.

To open a workspace your server account already has, for example on a new computer, ask "Open my workspaces on lab.example.edu." The chat checks the server first. When no saved connection names that server, a card asks to "Check bob@lab.example.edu as you: sign in, look at the Crew program in your account and list the workspaces there". Then the chat lists the workspaces, and opens the one you name after its own card.

A setup can wait for you to sign in while you are away from the chat. Ask any chat "What setups are waiting?", and it lists them, then shows the **Sign in** card for one when you ask.

### Which chats can

A chat gets the two tools when all of these hold:

- It is one of your own chats in the desktop app, or a terminal chat on the shared background service. A plain `biorouter session` in your terminal runs there whenever the background service is running, so it can use Crew as you ([Cards in a terminal chat](command-line.md#cards-in-a-terminal-chat)). An app's chat, a subagent, a scheduled run and a Crew task's chat do not get them.
- It has no Crew access. A chat connected with `/crew` keeps only its channel tools, and cannot act as you.
- The **Crew** switch in **Settings** > **Chat** > **Capabilities** was on when the chat started.

A chat on Claude Code or Codex gets both tools too, through the coding agent's tool bridge, and its changes ask you on the same cards. A change the coding agent sends again under the same call is applied once. Those two models are public, so such a chat reads only what a public model may read ([Privacy](#privacy-for-a-chat-that-reads-as-you)). Their tool bridge gives one call at most 10 minutes to wait for you, so there every card of one Crew request has to be answered within 9 and a half minutes of the request. A card still waiting then ends as if its time had run out, and nothing changes.

A workflow's chats get Crew too when the **Crew** switch is on, even when the workflow lists its own `extensions`: the capabilities you turned on for every chat stay on beside that list. A workflow that sets `capabilities: explicit` gets only its list, and its chat is told which capabilities are off there, so it can say so when you ask for one ([Workflow schema reference](../workflows/workflow-schema-reference.md#capabilities-turned-on-for-every-chat)). A chat that another chat opens with its own list of extensions keeps them too.

Without the background service, a terminal chat runs on its own and has no Crew tools. The chat is told "Crew actions as you need the desktop app or biorouter session --shared-daemon.", so it sends you there when you ask for Crew.

In a browser opened with `biorouter serve`, where Crew does not work, a chat gets `crew__read` only, and changes are not offered ([Getting started](getting-started.md)). The same holds for a chat in an editor that runs Biorouter through ACP (the Agent Client Protocol). Such a chat can still list your connections and setup jobs, but it cannot read a workspace's content, because the permission card below cannot be shown there.

The tools run only in the chat's own turn, where their cards can be shown. A program that calls them directly through the background service, outside a chat's turn, gets "crew__read and crew__change act as you, so they run only in a chat's own turn, where their approval cards can be shown. Use the Crew view or biorouter crew instead." With Code Execution on, as it is by default, the chat still calls `crew__read` and `crew__change` directly and never from a script. So each Crew read or change shows as its own row in the chat, each change has its own card, and a call that is sent again, for example after a dropped connection, is applied once. A script in a chat connected with `/crew` sees only `crew.connections` and `crew.request`.

The tool permission settings never list `crew__read` or `crew__change`, because each of their changes asks you every time, and a chat's own list there holds only the Crew tools that chat is offered.

### Reading needs your permission once

The first time a chat reads a workspace's content, it shows a card in the chat. Its first line reads "Let this chat read chen-lab as you", and the lines under it read "It can then read what you see in chen-lab on lab.example.edu (messages, files, names and the context document), and every change still asks you first. Take this back in this chat's Crew bar, or with biorouter crew grants revoke 20260924_2 --reads." Choose **Allow**. The card then reads "This chat may read chen-lab as you until you take it back". In a terminal chat, type `allow` at its prompt instead ([Cards in a terminal chat](command-line.md#cards-in-a-terminal-chat)).

If you choose **Deny**, the chat is told "The person declined. Nothing was read." It does not show the card again until you write in the chat: a read it tries before then answers "The person declined to let this chat read chen-lab as them, so nothing was read. Biorouter asks them again only after they write in this chat." When the card runs out, or you choose **Stop** in the chat, the read answers "This chat needs your permission to read chen-lab as you." (code `crew_read_consent_required`), and the next read asks again.

- The permission is for that chat and that connection, and lasts until you take it back. It does not expire after an hour, as a Crew grant does.
- Content is messages, files and their names, the workspace context, team, channel and people names, and task output. Lists of your connections, setup results and the server's workspace list carry none, so any chat reads them without asking. Downloading a file to this computer, and the state of a transfer that names its file, ask for this permission as reads do.
- In the chat itself, a bar above the message box reads "Reads chen-lab as you" for each workspace the chat may read, with **Revoke**. It appears as soon as you allow the card. Choose **Revoke**, and the bar says "This chat no longer reads chen-lab as you." **Dismiss** closes that note. If the permission was already gone, the note reads "This chat had no permission left to take back." If the chat reads chen-lab again later, its card asks again, and once you allow it the bar shows "Reads chen-lab as you" again, in place of the note.
- In Crew, type `/crew` in the chat. Under the Chat access pane, "Reads as you" lists "Reads chen-lab as you", with the date you allowed it, such as "Since November 6", and **Revoke**. Choose **Revoke**, and the pane says "This chat no longer reads chen-lab as you."
- From a terminal, `biorouter crew grants list` lists each permission, such as `Chat 20260924_2 reads chen-lab as you · since Nov 6, 2026`, and `biorouter crew grants revoke SESSION_ID --reads` takes it back ([A chat that reads as you](command-line.md#a-chat-that-reads-as-you)). You can also ask the chat to take it back. That is a change, so its card asks first, naming the chat as `grants list` does: “Take back permission for chat "Plot review" (chat 20260924_2) to read chen-lab on lab.example.edu as you”, or `chat 20260924_2` alone for a chat with no title. Its details say what it takes back: "permission to read as you".
- A chat that read Crew content keeps every other tool it has, unlike a chat with Crew access. With a private model it becomes Private: see [Privacy](#privacy-for-a-chat-that-reads-as-you).

### Every change waits for you

Each `crew__change` shows one card in the chat, with **Allow once** and **Deny**, and no **Always allow**. Its first line says what will happen, such as "Post this in #methods (Analysis Lab) in chen-lab on lab.example.edu as you", and the lines under it say what else matters, each on its own line. Nothing happens until you choose **Allow once**, and then exactly what the card names happens, once. After **Deny**, the chat is told "The person declined. Nothing was changed." A card waits up to 10 minutes, or 9 and a half minutes in a chat on Claude Code or Codex. If it runs out, or you choose **Stop** in the chat, nothing changes either. An answered card offers no **Change** link, because no standing permission applies to these tools: the next change asks again.

Three changes show a different card, with no **Allow once**, because what you type there is the act: signing in (`auth`), unlocking Crew's vault (`credentials_unlock`) and setting one up (`credentials_init`). See [Sign in, unlock and check a server's key](#sign-in-unlock-and-check-a-servers-key).

The card names its targets as Crew found them before it asked: every card names the workspace and the server it is on, such as "chen-lab on lab.example.edu", so two workspaces with a `#general` are told apart, then `#channel` with its team, a person as `Display name (@username)`, a computer's fingerprint. A card about a saved connection names it by the connection's name on this computer. It shows the whole payload: a message's full text, the workspace context as a change from the current one, a file's local path and size. Crew then acts on exactly the people, channels and computers the card named, and never looks a name up again.

| Action | The card says, for example |
|---|---|
| Posting (`send`) | "Post this in #methods (Analysis Lab) in chen-lab on lab.example.edu as you", with the whole message. One post can also carry up to 10 files from this computer and up to 10 files on the server, on one card: "Post this with 2 files from this computer and a reference to a file on the server in #methods (Analysis Lab) in chen-lab on lab.example.edu as you", then a line for each file, such as "plot.png (1.2 MB), from /Users/bob/results/plot.png." or "/project/results/summary.csv on the server.", and "Referenced files stay on the server; members who can read them there open them from the post. The post is restricted, so public models never read it." Crew uploads the files, then posts one message with all of them. |
| Sharing a file (`files_share`) | "Share plot.png (1.2 MB) from this computer in #methods (Analysis Lab) in chen-lab on lab.example.edu as you", then "From /Users/bob/results/plot.png." |
| Downloading a file (`files_download`) | "Download counts.csv (640 KB) from #methods in chen-lab on lab.example.edu to /Users/bob/Downloads/counts.csv on this computer". Without a destination, the file goes to your Downloads folder. |
| Starting a task (`tasks_ask`) | "Start an agent task in #methods (Analysis Lab) in chen-lab on lab.example.edu, on gpt-5.5 (versa_azure), with its own Crew access", then "It posts its result there as your agent.", "It reads only that channel." or "It may also read #data.", and what it may do on the server, such as "With a private model it can run commands in its work folder on hpc, confined to that folder." With full access it gives the full-access warning ([As me on the server](#as-me-on-the-server)). A chat at its usage limit, or any chat once the month is at its limit, gets no card and starts no task (`crew_usage_limit_reached`). The task's own spend counts toward its own task chat and the month, not the chat that asked. |
| Creating a channel (`channels_create`) | "Create #methods (Analysis Lab) in chen-lab on lab.example.edu as you", then "It is restricted: public models never read it." A new channel is Restricted unless the chat asks for public-safe, whose line is "It is public-safe: where the workspace allows Public, public models may read it." |
| Opening a workspace (`open`) | "Open chen-lab on bob@lab.example.edu, hosted by @alice, and join it as you", then "Workspace key fingerprint: …", the privacy, such as "Privacy: Private, institution ucsf.", and "You join it as its member." Joining from an invitation (`join`) names the same, and a line for each route or access choice you gave ([Set up Crew from a chat](#set-up-crew-from-a-chat)). |
| Setting Crew up on a server (`setup`) | Which workspaces it restarts and who is connected to them, such as "It restarts chen-lab: 2 other people are connected, and their running agent commands will stop." Without being asked, setup restarts only a workspace nobody else is connected to. The chat restarts one with others connected only when you name it, by the ID `setup_check` lists (`restart_workspace_ids`), or ask for every one (`restart_workspaces`), and the card says so. |
| Adding this computer again (`rejoin`) | "Its old key on this computer stops working." and that chats with Crew access through the connection each need access granted again. |
| Removing one of your computers (`profile_revoke_device`) | The Keys dialog's own warning: "A lost computer can still sign in as you while it has an SSH key or password for your account. First remove its key from ~/.ssh/authorized_keys on the server, or change your password. Then remove it here." For this computer, the card says instead that it stops reaching the workspace until you add it again, by asking a chat for `rejoin` or with **Add this computer again** in Crew. |
| Marking new computers as seen (`profile_devices_acknowledge`) | "Mark a new computer added to your account in chen-lab on lab.example.edu as seen", with each computer's fingerprint, the date it was added and how, then "Crew then stops pointing them out. If one isn't yours, deny this. Take its SSH access to the server away first, then remove it in Crew, under Keys." Reading your computers never marks them seen. |
| Drafting the workspace context (`context_draft`) | "Draft the context document for chen-lab on lab.example.edu from what the server reports about itself", then "Crew runs its fixed check of the server as you. It changes nothing there." and "The draft comes back to this chat. Nothing is saved until you approve saving it." The draft is workspace content, so the chat reads it as it reads other content. Saving it is `context_set`, with its own card. |
| Pausing, resuming or removing a transfer (`files_pause`, `files_resume`, `files_forget`) | "Pause the upload of counts.csv (640 KB) to chen-lab on lab.example.edu", then "It stops where it is, and you can resume it from there later." Resuming names the file's path on this computer, and its size for an upload. Removing reads "Remove from your list: the download of counts.csv (640 KB) from chen-lab on lab.example.edu", then "Removes the record on this computer." and, for an unfinished download, the partial file it deletes, such as "Its partial file is deleted from this computer: {path}." A transfer still under way is paused first. |
| Locking Crew's vault (`credentials_lock`) | "Lock Crew's vault on this computer", then "Every chat and Crew task using it stops until you unlock it." |
| Removing a workspace from this computer (`connections_remove`) | What it ends, and on a server with a current Crew, "The workspace can be opened again from any computer that signs in to the server as your account." |
| Taking back a chat's Crew access (`grants_revoke`) | “Take back Crew access to chen-lab on lab.example.edu from chat "Plot review" (chat 20260924_2)”, then "If it is a task, the task stops." Its details say what it takes back: "Crew access". |
| Deleting, archiving or removing for everyone | "for everyone; this cannot be undone", such as "Archive #methods (Analysis Lab) in chen-lab on lab.example.edu for everyone; this cannot be undone". |

If the chat gets Crew access while a card waits, the change does not run.

### Long work, and Stop

Setting up a server, hosting, opening or joining a workspace, adding a computer again, sharing or downloading a file and starting a task can take longer than one answer. The chat waits up to 20 seconds, then tells you the work goes on, and can follow it. Time a card spends waiting for you to sign in does not count toward those 20 seconds. **Stop** in the chat ends the wait, not the work: setups, transfers and tasks keep running and show in Crew, where you can stop them.

A setup a chat started appears at the top of Crew as "Crew is being set up on {server}.", or, when the server wants you to sign in, "A setup on {server} is waiting for you to sign in." with **Sign in**. You can sign in there or on the chat's own card.

### Sign in, unlock and check a server's key

Never type a password, code or passphrase into a chat. When a step needs one, or needs you to check a server's key, the chat shows a card for it. These cards have no **Allow once**, and nothing on them comes from the model: their words are Crew's own, with the server, login and fingerprint Crew itself read.

| Card | What to do |
|---|---|
| "Sign in to bob" | It reads "Sign in to bob on lab.example.edu. What you type goes to ssh on that server, never to this chat or the model. Type only into the Sign in window." Choose **Sign in**. The Sign in window opens, the same one Crew uses, and shows the server's own prompts. Type your password and verification code there. Once the server takes them, the card reads "Signed in. The chat carries on." and the chat goes on by itself. After **Dismiss**, the card reads "Dismissed. Nothing was signed in." If you signed in but Crew is not in your server account yet, the Sign in window says "Crew isn’t set up for your account on lab.example.edu. Biorouter can install it in your own account. It goes in ~/.local/bin and needs no administrator." Ask the chat to set Crew up ([Set up Crew from a chat](#set-up-crew-from-a-chat)). Where no Sign in window can open, the card says "Sign in is only in the desktop app. Open Crew there and choose Sign in, or run biorouter crew auth in a terminal. Never type your password into the chat." |
| "Unlock Crew's vault" | Choose **Unlock**. Your computer's own passphrase window opens, "Unlock Crew encrypted vault". Type the passphrase there. The card reads "Unlocked. The chat carries on." A refused passphrase says so on the card, and you can choose **Unlock** again. |
| "Set up Crew's vault" | The chat asks for one when this computer has no keyring Crew can use, such as a Linux computer you reach only over SSH. Choose **Set up**, type a new passphrase in "Initialize Crew encrypted vault", then type it again in "Confirm Crew vault passphrase". |
| "Can’t verify lab.example.edu yet" | This computer does not trust the server's key yet. The card shows "Fingerprint this computer read from the server". Get the server's fingerprint from your IT team and compare the two, character by character. If they match, choose **Trust this key**. Beside it the card says "Only trust it if IT or the server’s admin gave you this fingerprint." Crew reads the key from the server again and adds it to your known hosts file only when its fingerprint is the one on the card. You can also add the key yourself: under **How do I verify it?**, **Open a terminal here** opens a terminal on this computer for that ([Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer)). Once the key is there, either way, the chat goes on by itself. When you reach the server through a jump host, Crew reads no key from this computer, because it could reach another machine than the one SSH reaches. The card then usually shows no fingerprint and no **Trust this key**. Add the key yourself, and the chat goes on once any key for that server is there. |
| "lab.example.edu’s identity changed" | The server's key changed since this computer trusted it. Do not connect until your IT team confirms the change. Choose **Copy details for IT** and send them. Nothing waits, and the chat does not try again. This card has no **Trust this key**: Crew never trusts a changed key. The card stays in the chat's history, and read back later it says "A server's host key changed here, so Crew did not connect. Nothing connects to it until the server's IT team confirms the change." |

- **Dismiss** on the Sign in card, or **Not now** on a vault card or an unknown key's card, ends the chat's wait with nothing done. The chat is told so, and you can ask again later. A card waits up to 10 minutes, and **Stop** in the chat takes it down too.
- When a card stops waiting, it says how, and its buttons go: "This step is over. The chat carries on.", "Skipped. The chat carries on without this step.", "This step timed out before it was taken. The chat carries on without it." or "This step no longer waits for you. The chat carries on." This holds in every window that shows the chat, and after a reload: a card that ended somewhere else never comes back with live buttons. A card the chat drew before you opened it in a second window shows there too, and while one waits, the chat shows that it waits for you.
- A saved or shared chat shows a step that waited there as "A Crew step waited here. It is not waiting any more, so there is nothing to do."
- When the vault is locked and a change needs it, the vault card shows before the change's own card, so you never allow the same change twice. Changes that stop or take something back, such as removing a transfer or taking back a chat's access, do not wait for the vault. A read that finds the vault locked shows the vault card, then reads once you unlock it.
- After you add a server's key, the chat tries the step once more. A change you allowed is not asked again for that one try: your **Allow once** still counts if nothing on its card changed and you chose it less than 10 minutes before. Crew makes every other check again, and a card that would read differently asks you.
- If the server refused this computer's SSH key and your key has a passphrase, the chat can open the Sign in window for you to type that passphrase there.
- Crew adds a server's key only when you choose **Trust this key** for a key this computer does not know yet, and only the key whose fingerprint you checked. It never trusts a changed key, and the model can never trust one: only your own click, or the word `trust` typed in a terminal chat, does. The chat is told never to add, scan or accept a key itself. In **Completely Autonomous** mode, a shell command of the chat's that would trust a key nobody checked, such as `ssh -o StrictHostKeyChecking=accept-new` or the same option given to `sshfs`, asks you first ([Permission modes](../security/permission-modes.md#what-still-asks-whatever-your-mode)).
- While a card waits for a server's key, Crew looks for it in the known hosts files your SSH settings name for that login, not only `~/.ssh/known_hosts`. **Trust this key** adds it to the first of them.
- Locking the vault is an ordinary change: its card reads "Lock Crew's vault on this computer", then "Every chat and Crew task using it stops until you unlock it."
- Where no such card can be shown, as in a `biorouter serve` browser, where Crew does not work, or through ACP, the chat names the step instead: open Crew in the desktop app, or run `biorouter crew auth --setup-job ID`, `biorouter crew auth --connection NAME` or `biorouter crew credentials unlock`.
- In a terminal chat on the shared background service, the same steps run in your terminal: the server's own prompts for a sign in, a hidden prompt for the vault passphrase, and a question before it trusts a server's key. Each asks first, so a message you were typing never goes to a password prompt ([Cards in a terminal chat](command-line.md#cards-in-a-terminal-chat)).

For your first join, ask the chat to join. It connects the saved connection as needed. If the workspace does not know this computer yet, finish that first join before considering key recovery.

When this computer joined before and lost its key, or its registered key no longer works, choose **Add this computer again** in Crew, or let the chat do it with `rejoin` after you allow its card ([Add this computer again](joining-a-workspace.md#add-this-computer-again)). This replaces this computer's local device key. Its old key stops working. Chats using this connection need Crew access granted again.

### What a chat does shows in Crew

While Crew is open and its window is visible, a change made in a chat shows there within about two seconds, with no reload. So does a change made with `biorouter crew` in a terminal, or in another Biorouter window:

- A workspace a chat set up, hosted, opened or joined appears in the workspace list, and Crew selects it.
- A removed connection leaves the list.
- **Agent access**, "Reads as you" in the Chat access pane, and a chat's own Crew bar drop a permission a chat took back.
- The **Files** tab shows a transfer a chat paused, resumed or removed.
- The model for Crew tasks follows a change a chat made to it.
- **Add this computer again** leaves the connection bar once a chat added this computer again.
- A setup a chat started stays at the top of Crew while it runs. If it stops with an error, a note reads "Setting up Crew on {server} stopped." with Crew's reason, until you choose **Dismiss**.
- A chat's own Crew bar follows these changes even before this computer has any Crew connection, so the first workspace a chat sets up, and a permission given after it, show there at once.

Crew is told only which connection, channel, chat or job changed, never what changed in it, and reads the rest itself.

In the chat itself, each Crew row says what it did, such as "Posting in #general", "Creating #methods in Analysis Lab" or "Starting a task in #qc", with names from Crew's answer, never an ID. A finished change in your own chat also has **Open in Crew**, which opens Crew on that workspace and channel and highlights the message or task. A started task also has **Open task chat**, which opens the task's own chat. **Open in Crew** only selects: it never connects, so a workspace that is offline opens offline until you choose to connect it. A saved or shared chat shows neither button. Clicking a Crew notification opens Crew on its workspace and channel the same way.

### Privacy for a chat that reads as you

The chat follows the same privacy rules as a chat with Crew access, channel by channel:

- A public model reads only a Public workspace through a Public connection, and only its Public-safe channels. Restricted messages and files are withheld, and the chat is told how many. A task's output that is private is withheld from it too: "This task's output is private, so a public model can't read it. Its status is shown."
- A private model must be approved for the workspace's institution, or run locally. No chat reads a Private workspace that has no institution, or reads through a Private connection that has none, or through a connection whose institution is not the workspace's.
- A chat that already holds private content of one institution reads no workspace of another institution, nor one with no institution, as a Crew grant into that chat would be refused. Start a new chat to read it.
- With a private model, reading anything that is not public-safe makes the chat Private before the answer reaches it. If Crew cannot make it Private, the chat gets nothing.
- A post or a file a Private chat shares is marked Private, and its card says so: "It is marked Private, so public models never read it." A connection whose privacy is Public could not keep it private, so the chat refuses instead (code `crew_mode_mismatch`).
- When a Private chat writes into a Public workspace, through a Public connection, or into a public-safe channel, whether a post, a file, a team or channel name, the workspace context or your profile, the card adds a line such as "This chat is Private, and this goes to a public-safe channel: a private-to-public crossing. Everything it sends is shown here." and shows the whole payload.
- A task a Private chat starts is Private too, and its card says "This chat is Private, so the task is too: a public model is refused."
- The chat's lists of chats with access, and of tasks, show only the chats it could list itself, so a chat on a public model never sees a Private chat there. A grant's channel names show only for a workspace the chat may read as you, and a chat on a public model sees only the public-safe ones. Refusals never name a workspace, channel or person the chat may not see.

### When the chat says it cannot

A refusal the chat reads ends with its code, such as "(code: crew_read_consent_required)", and, when a step needs you, that step.

| The chat says | Why, and what to do |
|---|---|
| "This chat needs your permission to read {workspace} as you." | The read card ran out, or you chose **Stop** before answering it. Ask again, and choose **Allow**. |
| "The person declined. Nothing was read." | You chose **Deny** on the read card. |
| "The person declined to let this chat read {workspace} as them, so nothing was read. Biorouter asks them again only after they write in this chat." | The chat tried to read again after you chose **Deny**. Write in the chat, and the next read shows the card again. |
| "This chat already holds private content that belongs to another institution, so it can't read this workspace too. Start a new chat to read it." | Code `crew_institution_mismatch`. The chat read another institution's private content before, so it cannot read this workspace, or one with no institution. Start a new chat. |
| "Biorouter couldn't read which institutions this chat's content belongs to, so it read nothing from the workspace. Try again." | Ask again. |
| "This chat uses a public model, so it can't read this workspace as you. Its connection and the workspace both have to be Public. Choose a private model to read it." | Code `crew_public_model_refused`. Use a private model, or read it in Crew. |
| "This workspace has private or restricted content and no institution, so a chat can't read it. Its host can set one with privacy_set_workspace, or read it in the Crew view." | The host marks the workspace with its institution ([Mark the workspace with your institution](hosting-a-workspace.md#mark-the-workspace-with-your-institution)). |
| "This connection is Private and has no institution set, so a chat can't read through it. Set one with privacy_set_personal." | Set **Institution** in **Connection settings…**. |
| "This connection and its workspace are set to different institutions, so a chat can't read it. Set the connection's institution to the workspace's with privacy_set_personal." | Code `crew_institution_mismatch`. Use the workspace's institution in **Connection settings…**. |
| "Biorouter couldn't mark this chat Private, so it returned nothing from the workspace. Try again." | Ask again. |
| "This chat is Private, and your connection to {workspace} is Public, so what it writes there couldn't be kept private. Write it from a chat that isn't Private, or make the connection Private with crew__change action privacy_set_personal." | Code `crew_mode_mismatch`. Post from a chat that is not Private, or make the connection Private in **Connection settings…**. |
| "This chat can't download that file: it is restricted, or in a channel this chat can't read." | Download it in the **Files** tab, or from a chat on a private model. |
| "Nothing was changed: the chat was stopped before it ran." | You allowed the card, then chose **Stop** before the change ran. Ask again. |
| "{connection} changed while this waited for your approval, so nothing was changed. Look at it again with crew__read action connections_show, then ask again." | The saved connection changed while the card waited, for example in **Connection settings…**. Ask again, so the card shows it as it is now. |
| "{file} changed on this computer while this waited for your approval, so nothing was shared. Ask again to share it as it is now." | The file changed after the card showed it. Ask again. |
| "That channel belongs to its team. Delete the team instead, with crew__change action teams_delete." | A team's `#general` goes only with its team. |
| "Open a workspace from a server checked in this chat: run crew__read action setup_check with the server first." | `open` takes the workspaces that a chat's `setup_check` of a server login found, once the check finished. A check run in the Crew view or a terminal, or before the background service restarted, does not count. Ask the chat to check the server, then to open the workspace. |
| "Reading a workspace as you needs your permission once per chat, which this Biorouter can't ask you for here. Read it in the Crew view of the desktop app or with biorouter crew in a terminal." | Code `crew_read_consent_required`. The chat runs where the permission card cannot be shown ([Which chats can](#which-chats-can)). Read it in Crew in the desktop app, or from a terminal. |
| "This needs your approval, which this Biorouter can't ask you for here. Do it in the Crew view of the desktop app or with biorouter crew in a terminal." | The chat runs where no card can be shown ([Which chats can](#which-chats-can)). Make the change in Crew in the desktop app, or from a terminal. |
| "The person closed the Sign in card, so nothing was signed in." | You chose **Dismiss**. A setup that waited for you still waits. Ask the chat again when you can sign in. |
| "Nobody signed in before the Sign in card's time ran out." | The card waited 10 minutes, or as long as the setup still had. Ask again. |
| "No Sign in window can open for this chat." | The chat runs where no card can be shown. Sign in from the Crew view of the desktop app, or with `biorouter crew auth --setup-job ID` or `biorouter crew auth --connection NAME` in a terminal. |
| "Nothing waits for a sign-in: it is connected." | The connection needs no sign-in now. For a setup that no longer waits, the chat says "Nothing waits for a sign-in: this setup isn't waiting for one.", and for a connection that is not connected, "Nothing waits for a sign-in. Connect it with crew__change action connect, which shows a Sign in card if its server asks for one." |
| "Name a setup job or a saved connection to sign in to, not both." | The chat named both for one sign-in. Ask again, naming the setup or the workspace. |
| "The person closed the card without unlocking Crew's vault, so it is still locked." | You chose **Not now**, or closed the passphrase window. Ask again, or unlock it in **Keys and security…**. |
| "This chat can't open the vault prompt here." | The chat runs where no card can be shown. Unlock the vault in **Keys and security…** in Crew, or with `biorouter crew credentials unlock`. |
| "Crew has no vault on this computer, so there is nothing to unlock. It keeps its keys in this computer's keyring." | Nothing is locked. Crew keeps your keys in the system keychain. |
| "{host}'s host key fingerprint is {fingerprint}." | Code `crew_ssh_host_key_unknown`: the server's key is not in your known hosts file yet. Check the fingerprint with IT, then choose **Trust this key** on the chat's card, or add the key yourself ([Sign in, unlock and check a server's key](#sign-in-unlock-and-check-a-servers-key)). With a changed key (code `crew_ssh_host_key_changed`), the chat names both fingerprints. Do not connect until IT confirms the change. |
| "This chat can't resume that upload, because it may not see which file it was sharing. Resume it in Crew, under Files." | The chat may not be told which file the upload was sharing, for example because it is restricted and the chat uses a public model. Resume it in the **Files** tab. |
| "Crew actions as you are available only in your own chats, not in an app, a subagent or a scheduled run." | The chat is not one of your own ([Which chats can](#which-chats-can)). Ask in one of your own chats. |
| That this computer already has a connection to the workspace, and joining uses it as it is | You gave a login, route or agent access to join a workspace this computer already has. Ask the chat to change the saved connection (`connections_update`, `connections_set_access`), then to join it. |
| "{path} is not a file. Name each file to share, not a folder." | A post or a share named a folder. Name each file. |
| "One message carries at most 10 files. Send the rest in another message." | Post the rest in a second message. The same holds for references to files on the server. |

### Do it in Crew, in a terminal, or in a chat

Each row is one task, with the Crew view's control, the command, and the chat action that does it. Read actions use `crew__read`, and the others `crew__change`. `crew__read` action `describe` tells the chat any action's parameters.

| Task | Crew view | Command line | Chat action |
|---|---|---|---|
| See your connections | Workspace menu, status row | `status`, `connections list`, `connections show` | `status`, `connections_show` |
| Read an invitation before saving it | Join dialog summary | `connections join-invitation --preview` | `connections_preview` |
| Check a server, and list its workspaces | **Open a workspace on a server** | `setup --check`, `doctor` | `setup_check` |
| Set up or update Crew on a server | **Set up Crew for my account**, **Update Crew** | `setup` | `setup`, `setup_status`, `setup_cancel` |
| See the setups on this computer, from any chat, Crew or a terminal | The notes at the top of Crew | No command: `setup` follows only its own | `setup_list` |
| Sign in to a server | **Sign in** | `auth`, `auth --setup-job ID` | `auth`, which shows a **Sign in** card |
| Crew's vault | **Keys and security…**, **Unlock** on the connection bar | `credentials status`, `credentials init`, `credentials unlock`, `credentials lock` | `credentials_status`, `credentials_init` and `credentials_unlock` (each a card that opens your computer's passphrase window), `credentials_lock` |
| Host a workspace | **Host a new workspace** | `host` | `host` |
| Open a workspace your account has | **Open a workspace on a server** | `open` | `open` |
| Join with an invitation | **Join a workspace**, **Join {workspace}**, its **Advanced** route | `connections join-invitation` (with `--ssh-target`, `--port`, `--identity-file`, `--proxy-jump`), `join` | `join` (with `ssh_target`, `port`, `identity_file`, `proxy_jump`, `folder`, `agent_access`) |
| Trust a server's key you checked | **Trust this key** on "Can’t verify {server} yet" | No command: add the key in your own terminal | **Trust this key** on the chat's card; the model cannot do it |
| Add this computer again | **Add this computer again** | `rejoin` | `rejoin` |
| Connect or disconnect | **Connect to {workspace}**, **Disconnect** | `connect`, `disconnect` | `connect`, `disconnect` |
| Change a connection | **Connection settings…** | `connections update`, `connections set-access` | `connections_update`, `connections_set_access` |
| Remove a workspace from this computer | **Remove {workspace} from this computer…** | `connections remove` | `connections_remove` |
| See the workspace and its people | "{workspace} settings", **People…** | `workspace show`, `members` | `workspace_show`, `members` |
| Rename, delete or govern the workspace | **General** tab | `workspace rename`, `workspace delete`, `workspace governance` | `workspace_rename`, `workspace_delete`, `workspace_governance` |
| Read, draft or save the workspace context | **Context** tab, **Draft from server** | `workspace context show`, `workspace context draft`, `workspace context set` | `context_show`, `context_draft`, `context_set` |
| Invite, cancel, let in or remove a person | **Invite people to {workspace}…**, **Cancel invitation**, **Let in…**, **Remove from {workspace}…** | `enroll invite`, `enroll cancel`, `enroll approve`, `enroll revoke`, `enroll pending` | `enroll_invite`, `enroll_cancel`, `enroll_approve`, `enroll_revoke`, `enroll_pending` |
| Show an invitation again (host) | **Invite people to {workspace}…** | `connections invitation --for @bob` | `connections_invitation` |
| Add people to teams and channels | **Add people to {team}…** | `members add` | `members_add` |
| Teams | **Create team**, **Rename…**, **Join {team}**, **Delete team…** | `teams list`, `teams create`, `teams rename`, `teams join`, `teams delete` | `teams_list`, `teams_create`, `teams_rename`, `teams_join`, `teams_delete` |
| Channels | **Create channel**, **Rename…**, **Archive channel…**, **Browse channels…**, **Delete channel…** | `channels list`, `channels create`, `channels rename`, `channels archive`, `channels join`, `channels delete`, `channels mark-read` | `channels_list`, `channels_create`, `channels_rename`, `channels_archive`, `channels_join`, `channels_delete`, `channels_read` |
| Team and channel invitations | **Join {team}** | `invites list`, `invites create`, `invites accept` | `invites_list`, `invites_send`, `invites_accept` |
| Hand over a channel, or remove someone from it | **Transfer ownership…**, **Remove** | `ownership offer`, `ownership accept`, `remove-member` | `ownership_offer`, `ownership_accept`, `remove_member` |
| Read and search messages | The channel | `history`, `watch`, `search` | `history` (with `after` to follow), `search` |
| Post | The message box | `send` | `send` (with `attachments` and `references`, up to 10 of each) |
| Files | **Upload a file…**, **Share**, the **Files** tab | `files list`, `files status`, `files upload`, `files download`, `files reference` | `files_list`, `files_status`, `files_share`, `files_download`, `files_reference` |
| Transfers under way | The **Files** tab: **Pause**, **Resume…**, **Remove from list** | `files pending`, `files pause`, `files resume`, `files forget` | `files_pending`, `files_pause`, `files_resume`, `files_forget` |
| Tasks | **Ask my agent**, `@agent`, **Stop** | `ask`, `tasks start`, `tasks list`, `tasks watch`, `tasks cancel` | `tasks_ask`, `tasks_list`, `tasks_watch`, `tasks_cancel` |
| Chats with access, and permissions to read | **Agent access** tab, **Revoke**, "Reads as you" in the Chat access pane, **Revoke** in the chat's own bar | `grants list`, `context SESSION_ID`, `grants revoke` | `grants_list`, `grants_revoke` |
| Privacy | **Privacy…**, **Connection settings…** | `privacy show`, `privacy set-personal`, `privacy set-workspace` | `privacy_show`, `privacy_set_personal`, `privacy_set_workspace` |
| Your profile and computers | **Edit profile…**, **Keys and security…**, **Acknowledge** | `profile show`, `profile set`, `profile devices`, `profile devices --acknowledge`, `profile revoke-device` | `profile_show`, `profile_set`, `profile_devices`, `profile_devices_acknowledge`, `profile_revoke_device` |
| The model for Crew tasks | **Settings** > **Models** > **Crew tasks** | `defaults show`, `defaults set`, `defaults clear` | `defaults_show`, `defaults_set`, `defaults_clear` |

`setup_check` against a server login that no saved connection names with the same port, key file and jump host, `open` and `join` are changes, so they show a card too. `open` needs a finished `setup_check` of the server login from a chat first ([When the chat says it cannot](#when-the-chat-says-it-cannot)). When the Crew in your account is too old to list the workspaces and this Biorouter can update it, the chat's answer names the chat's own way to do that, such as "Crew 1.93.0 in your account on lab.example.edu is too old to list workspaces. Update it with crew__change action setup, which then lists them."

A few things stay with you, on purpose, and no chat does them:

- Starting and stopping the background service, which the chat itself runs in.
- The steps the setup and join flows run for you: `workspace bootstrap`, `enroll prepare`, `enroll accept`, `connections prepare` and `connections save`.
- Giving a chat Crew access: type `/crew` ([Connect a chat with /crew](#connect-a-chat-with-crew)). A chat can start a new task for you with its own access, after you allow the card that names its channel, model and server access.

A chat's own shell cannot go around the cards. When a chat runs `biorouter crew` in its shell, reads work as they do for you, and every change is refused before anything is sent, with "An agent's shell can't change Crew as you. Use crew__change in the chat, which asks the person first." and the action to use, such as "The action is send." (code `crew_agent_shell_refused`). The chat then makes the change with `crew__change`, and you see its card. The steps above that stay with you are refused with "An agent's shell can't change Crew as you. Only you can do this, in the Crew view or with biorouter crew in your own terminal." This guards against a mistake, not against a program set on getting around it ([Privacy and security](privacy-and-security.md)).

## What your agent may do on the server

Each connection has an agent access level, which says what your agents may do on the lab server. You choose it for your own account when you join, and change it in **Connection settings…**, under "What can your agent do on {server}?". It applies to every task and connected chat through that connection, and only with a private model. A public model gets none of it, whatever the level.

| Level | What your agent may do |
|---|---|
| **Off** | "It can’t use files or run commands on {server}." |
| **In a folder** | "It reads and writes files in one folder, and runs confined commands there." See [Work in the remote work folder](#work-in-the-remote-work-folder). |
| **As me on the server** | Full access: "It runs commands as {you}, with your files, network and programs." See [As me on the server](#as-me-on-the-server). |

A new connection made by joining or hosting starts at **As me on the server**, with the warning below, and you may choose less. **Off** stays off: setting Crew up again never raises it, except that `biorouter crew join` on a connection that never connected from this computer gives it the new-connection level and says so ([Joining](joining-a-workspace.md)). **As me on the server** needs a current Crew on the server, in the host's account and in yours, and is unavailable, with "Update Crew on {server} first.", until it is there. `biorouter crew setup --replace-existing` updates your own copy, and the host updates the workspace's ([Upgrade Crew](administration.md#upgrade-crew)). From a terminal, `biorouter crew connections set-access off`, `folder` or `full` sets the level ([Agent access](command-line.md#agent-access)).

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

> **Warning.** Crew shows this before you choose it: "Your agent will run commands as {you} on {server}: it can read and change anything your account can, use the network, and start programs that keep running after access ends. Other people’s messages and files in channels it reads can try to steer it. Use a private model you trust. Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark." If you host the workspace from that account, the warning adds "You host {workspace} from this account, so your agent can also read the workspace’s own files on the server, including channels you’re not in. It can also act as the host of {workspace}." See the end of this section.

Every other way to set full access, or to start a task with it, gives the same warning:

- For a connection with full access, **Ask my agent** says "Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark." in its footer, under the line that names what the task may read, before you start the task. The message box says the same under its access line when you start a task with `@agent`.
- In a terminal, `setup`, `open` and `join` print it when they give your agent full access, `host` prints it after the list of what it will do, and `connections set-access full` asks with it ([Agent access](command-line.md#agent-access)). `tasks start` and `ask` print it after the task they started on a connection with full access.
- A chat's card for setting Crew up, opening a workspace or hosting one says "Agents get full access: they can run any command as you on lab.example.edu, read and change any file your account can, and use the network. Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark."
- A chat's card for `connections_set_access` with full access says what it allows, then "Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark." on a line of its own.
- A chat's card for a task (`tasks_ask`) whose connection has full access says "With a private model it can run any command as you on lab.example.edu, and read and change any file your account can. Crew treats anything running as your account as you, so your agent can also add a computer to your account and post in Crew as you, with no agent mark."

On the account that hosts the workspace, each adds the same sentence, such as "You host chen-lab from this account, so your agent can also read the workspace's own files on the server, including channels you're not in. It can also act as the host of chen-lab." When Biorouter cannot tell whether you host the workspace, it leaves that sentence out: for example when a saved connection is not connected, or a chat's card gets no answer from the workspace within 10 seconds.

What the agent can do there:

- **Run a command.** `remote.shell` runs one command in your login shell. Crew waits up to 20 seconds for it (10 by default), then answers with its output, or with a job the agent checks with `remote.job_status`. A command may run for up to an hour. A longer one runs as a detached job (`detach`), which keeps running after the task ends, and the agent or a later task checks it with `remote.job_status` or stops it with `remote.cancel`.
- **Work with files.** It lists, reads, writes and checks files anywhere your account can, by full path, `~/` path or a path in the work folder, up to 384 KB per read or write.
- **Copy a channel's file to the server.** `remote.fetch_blob` copies a shared file of up to 1 GB to a path on the server, in the background, and checks its SHA-256. The task's Source line then names it as read.
- **Post a server file to the channel.** `remote.share_file` uploads a server file of up to 1 GB in the background, and Crew posts it in the task's channel when the copy is done.

Crew follows each copy for up to 20 seconds and settles it before it answers the agent. A copy still running after that is answered "Still copying. Check it with remote.job_status before you finish; Crew posts the file when it is done.", and the agent is told to check it before it finishes. One account runs at most 8 commands and copies at once.

Every command the agent runs is announced in the task's steps by its program and its length, such as "Requested remote.shell: Rscript (52 characters; runs as your account)", never its text. The secret guard still refuses commands and files that would read a credential, and results that carry one are withheld ([Files Crew will not share](privacy-and-security.md#files-crew-will-not-share)).

The agent is told to run heavy computation on the server rather than in its reply: it writes a script with `remote.write`, runs it with `remote.shell`, and starts long work as a detached job. It reads the [workspace context](#the-workspace-context) first, to learn the server's storage, software and how jobs run there.

Ending access, by revoking a chat or when a task finishes, stops new commands, but a detached job keeps running in your account. Check it, or stop it yourself with `kill` on the server.

Crew treats every program running as your server account as you, because that is how it knows you ([You are your server account](joining-a-workspace.md#you-are-your-server-account)). So full access lets a command of your agent's add a computer to your account in Crew and post as you, with no "agent" mark, and, on the account that hosts the workspace, act as its host. Crew's own program on the server refuses to add or remove your computers from inside an agent's job ("An agent's job on the server cannot add or remove your computers. Do this from Biorouter on your computer."), but that is a guard against mistakes, not a barrier: a command that reaches the workspace another way is not stopped. Give full access only with a model you trust, and check your computers in **Keys and security…** ([Your computers](privacy-and-security.md#your-computers)). An agent **In a folder** cannot reach the workspace at all.

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
| "model is fixed by its Crew access", "already has Crew access to another channel", "saved on this computer", "another institution's context", "cannot be rebound to a public model" | Move the work to a new chat, as in [A chat's fixed channel and model](#a-chats-fixed-channel-and-model). |

## See which agents have access

- **Agent access** tab: channel menu > **Agent access**, or the channel header chip ("2 chats", "1 task", "3 agents").
- The **Agent access** tab of "{workspace} settings": workspace menu > **Agent access…**.
- Agents section of the Crew sidebar, while a task runs or a chat has access. A chat row opens its Chat access pane.

"+2" on a row means it reads two more channels. Every row has **Open**. An active chat has **Revoke**, a running task has **Stop**, and an unconfirmed revoke has **Retry**.

| Badge | Meaning |
|---|---|
| "Active · ends 4:40 PM" | Can read and post |
| "Stopped on this computer" | Revoked here, not yet confirmed by the workspace |
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

"Stopped on this computer" means the workspace was unreachable. The chat is already stopped here. Crew keeps asking the workspace, even after a restart, and shows "Confirmed. The workspace has stopped this chat’s access too." when it answers. **Retry** asks at once. If the workspace still cannot be reached, it says "Still can’t reach {workspace} · checked just now. Connect to confirm it now.", with **Connect** where the screen has one. In the chat, one note names the workspace, with **Retry** and **Start a new chat**.

"Not revoked. This chat can still read and post." is followed by Crew's reason. Revoke again, or revoke from the workspace the chat is connected to.

## After access ends

The chat's note says whether access was removed, expired after an hour (even offline), or ended because Crew settings changed. Sending, editing, retrying and queued messages stop, and Enter shows "Can’t send". A running reply ends with "Crew access removed" or "Crew access ended". Because the chat holds channel messages, it cannot continue as an ordinary chat.

- **Grant access again** opens the consent on the chat's channel. Allow gives up to one more hour.
- **Start a new chat** opens a chat with no Crew access and no limits. The old chat stays in your history.

### Why settings changes end access

Access ends when:

- your connection's privacy, settings or institution change, or you add this computer again with **Add this computer again**;
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
