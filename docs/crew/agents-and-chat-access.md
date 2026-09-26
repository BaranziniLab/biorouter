# Agents and chat access

> **What this is.** How to let an AI agent read a Crew channel and post in it, with **Ask my agent** or by connecting an ordinary Biorouter chat with `/crew`. It also covers how to follow a task, how to see and revoke access from every place that shows it, and what a chat shows after its access ends.
> **Status:** Current.
> **Audience:** Lab members who use Crew in the Biorouter desktop app. You do not need a terminal for anything on this page except the short command line section.

Crew calls two things agents. A task is a single job you give your agent from a channel with **Ask my agent**. A connected chat is an ordinary Biorouter chat that you connect to a channel by typing `/crew` in it. Both read the channel and post in it as your agent, and everyone in the channel sees what they post. Nothing can read or post until you allow it. Access lasts one hour at most, and you can end it at any time. The Biorouter background service on your computer and the workspace on the server decide every start, grant and revoke. The buttons on screen only send requests to them.

## Tasks and connected chats compared

| | Task | Connected chat |
|---|---|---|
| Where you start it | **Ask my agent** in the channel's message box | Type `/crew` in an ordinary Biorouter chat |
| What you give it | One written task | Your conversation, message by message |
| What it can read | The channel, plus any channels you tick under **Also read** | The same |
| Where it can post | Only that channel | Only that channel |
| Who sees its posts | Everyone in the channel | Everyone in the channel |
| When its access ends | When the task ends, and never later than one hour after it starts | When you revoke it, one hour after you allow it, or when Crew settings change |
| How you end it | **Stop** on the task's row | **Revoke access** |

## Before you start

- You need at least one AI model set up in Biorouter. Crew does not walk you through model setup. If none is set up, the Ask my agent pane says "No models are set up." and offers **Open Settings**.
- Some workspaces and channels accept only Private models. See [Which models Crew accepts](#which-models-crew-accepts).
- One computer can run four tasks at the same time. A fifth is refused until one ends.
- Only you see your own agents in the lists on this page. Other people's agents never appear there, even when they post in your channels. In a channel, another person's agent appears under their name with "'s agent" added, for example "Gina Rossi's agent".

### What a Private workspace or connection also needs

Crew needs three more things when the workspace or your connection is Private, or when the agent reads a Restricted channel. [Privacy and security](privacy-and-security.md#which-models-an-agent-may-use) lists every case. Crew checks the first two only when you choose the start or Allow button. If one is missing, Crew refuses with a message.

1. The host has set the workspace's institution. Otherwise Crew refuses with "Confirm this workspace's institution before granting an agent; unlabelled private workspaces allow human collaboration only". Only the host can set it. See [Hosting a workspace](hosting-a-workspace.md#mark-the-workspace-with-your-institution).
2. If your connection is Private, it has an institution too. Otherwise Crew refuses with "Set this private SSH connection's institution before granting an agent". Set it in **Connection settings…**. See [Connections and troubleshooting](connections-and-troubleshooting.md#connection-settings).
3. The model is approved for that institution, or runs on your computer.

An approved model is one whose provider carries your institution. Biorouter takes the institution from the provider. You cannot mark a model as approved yourself. In the Ask my agent model list, an approved model's chip names the institution, for example "Private · UCSF". To set up such a provider, open **Settings** > **Models**. Institutional providers are under the **Institutional** tab of the provider list.

A local model runs on your computer. Examples are Llama Server, and Ollama when it runs on your computer. Its chip reads "Private · On this machine". Crew accepts a local model for every institution, but items 1 and 2 still apply.

## Ask my agent

### Start a task

1. Open the channel where the result should appear.
2. If you like, type the task in the channel's message box first. Crew copies it into the task form once, when the form opens.
3. Choose **Ask my agent**. It has a robot icon and sits in the bottom row of the message box, to the left of the send button.
   The Ask my agent pane opens on the right side of the window, and the cursor is in the **Task** box.
4. Read the first line of the pane, for example "Posts to #methods in Analysis Lab · lab". It names the channel, team and workspace that will receive the result.
5. Type the task in **Task**. Press Enter to start a new line. Enter does not start the task.
6. Check **Model**. To use another model, choose **Change**.
7. If the agent needs other channels, open **Advanced** and tick them under **Also read**.
8. Read any notes above the start button.
9. Choose **Start my agent and allow posting here**.

The button reads "Starting…" with a spinner until Crew accepts the task. The pane then closes and the task appears in the channel. Crew scrolls to the task and highlights it once. If the message box still holds the text that filled the task, Crew clears it.

To close the pane without starting, choose the × (its name is "Close Ask my agent") or press Escape. Choose **Ask my agent** again to open or close the pane. In a narrow window the pane covers the channel and shows **Back to #methods** at the top.

### What the pane shows

From top to bottom:

| Part | What you see | What it tells you |
|---|---|---|
| Destination | "Posts to #methods in Analysis Lab · lab" | Where the task and its result are posted. |
| File note | "2 files named gina-assay.csv are shared in #methods. Your agent will use the newest one, shared yesterday at 1:55 AM." | Shown only when the task names a file that several shared files have. See [Files named in the task](#files-named-in-the-task). |
| **Task** | An empty box with "What should your agent do?" | Your instructions. |
| Hint under **Task** | "Your task is posted in #methods so everyone there can see what your agent was asked." | Always shown. For a task longer than 10 lines or 1000 characters it adds "Long pasted data will be visible to the channel." |
| **Model** | The model name, its provider, a privacy chip and **Change** | The model that runs the task. See [Choose a model](#choose-a-model). |
| **Advanced** | Closed, with a summary such as "Reads only #methods" or "Also reads #qc, #imaging" | Other channels the agent may read. Shown only when the workspace has other channels or your connection has a work folder. |
| Public model warning | "This model is Public, so it can’t read Restricted channels." | Shown when a Public model is chosen and a channel it would read is Restricted. |
| Scope line | "Your agent can read #methods and post there, for this task only." | Always shown. The access ends when the task ends. |
| Warnings | The institution warning, or "No file named … is shared in #methods." | See [Which models Crew accepts](#which-models-crew-accepts) and [Files named in the task](#files-named-in-the-task). |
| Error | A red note with Crew's reason | Shown when Crew refused the start. See [If Start is refused](#if-start-is-refused). |
| **Start my agent and allow posting here** | The start button | Starts the task and lets it post in this channel. |

The start button is greyed out when:

- A start is in progress.
- Crew is still verifying the channel.
- The channel is archived.
- No models are set up.
- The chosen model is not approved for the workspace's institution.
- The [unknown outcome warning](#if-crew-cannot-tell-whether-a-task-started) is open.

### Your task is posted in the channel

Before the agent begins, Crew posts your whole task in the channel as a message that starts with "Task:". Everyone in the channel can read it, including anything you pasted into it. Do not paste data you would not post in that channel yourself.

In the channel, you see the task and its result under "Your agent", with an "Agent" badge. Other people see the same posts under your name with "'s agent" added, for example "Alice Chen's agent".

### Choose a model

The **Model** area has three forms.

- When Biorouter has a default model, you see it as the model name, a dot, and the provider name, then a privacy chip and **Change**. Choose **Change** to open the list.
- When no model is set up, you see "No models are set up." Choose **Open Settings** to go to the models section of Settings. Set up a model there, then come back.
- Otherwise you see a field that reads "Choose a model". Choose that field to open the list.

In the list:

- The cursor starts in the **Search models** box. Type part of a name to filter. "Loading models…" shows while the list loads, and "No models match." when nothing matches.
- Models are grouped by provider. Only providers that are set up in Biorouter appear. A check mark shows the current choice.
- To use a model that is not listed, type its exact name. The list then offers "Use “name” with provider" for each provider.
- Use the arrow keys to move, Enter to choose, and Escape to close the list.

If you choose the start button before you pick a model, the pane says "Choose a model." under the field and moves the cursor there.

Each provider carries a privacy chip:

| Chip | Meaning | Text when you point at it |
|---|---|---|
| A padlock chip that reads "Private · UCSF" | A Private model, approved for the institution named after the dot | "Private. Only private models can run this task." when this task needs a Private model. Otherwise "Private. Unlike a public model, it may read Restricted channels." |
| "Public" | A Public model | "Public. It can’t read Restricted channels." |
| A padlock chip that reads "Private · On this machine" | A local model, which runs on your computer | The same texts as any Private model. Crew accepts it for every institution. |
| A padlock chip that reads "Private · No stated institution" | A Private model whose provider names no institution | The same texts as any Private model. Crew refuses it when the task needs a Private model. |
| "Not approved for UCSF" under a provider or a model | The workspace's institution has not approved this model | Crew will not start the task with it. |

When Crew cannot tell a model's privacy, the chip shows only the institution, or nothing.

### Which models Crew accepts

Crew checks the model before the task starts. For a connected chat, Crew checks the chat's model when you choose Allow. The same rules apply to chats, but some refusals use other words. See [If Allow is refused](#if-allow-is-refused). A chat that was connected before keeps its first model. See [A chat keeps its first channel and model](#a-chat-keeps-its-first-channel-and-model).

| Situation | What Crew does |
|---|---|
| The model is Public and the channel, or a channel under **Also read**, is Restricted | The pane warns "This model is Public, so it can’t read Restricted channels." Crew refuses the start. Choose a Private model. |
| The model is Public and the workspace or your connection is Private, or your connection has a work folder on the server | Crew refuses the start with a message such as "Private workspace blocks public models". The pane may not warn you first. Choose a Private model. |
| The model is Private but approved for another institution | The pane shows "{model} is approved for {institution A}. {workspace} uses {institution B}. Choose a model approved for {institution B}, or a local model." The start button is greyed out. |
| The model is Private but does not say which institution approved it | The pane shows "{model} doesn’t say which institution approved it. {workspace} uses {institution}. Choose a model approved for {institution}, or a local model." The start button is greyed out. |
| The model runs locally on your computer | Local models are accepted for every institution. |
| The provider is Claude Code or Codex | Crew refuses the start with "This provider controls external tools that Crew cannot isolate. Choose a provider using BioRouter's scoped tools." For a chat, Crew refuses Allow with "Crew cannot admit providers with external tools outside its scoped capability boundary". These providers still appear in the list. Choose another provider. |

Choosing another model clears an institution error. For what Private, Public and Restricted mean, see [Privacy and security](privacy-and-security.md).

### Let the agent read other channels

**Advanced** appears when the workspace has other channels that are not archived, or when your connection has a work folder on the server.

1. Choose **Advanced**.
2. Under **Also read**, tick each channel the agent may read. The list covers every team in the workspace. A channel reads "#qc", or "Team / #qc" when two teams have a channel with that name.
3. Close **Advanced** if you like. Its summary then reads, for example, "Also reads #qc, #imaging", or "Also reads #a, #b, #c and 2 more".

You can tick up to 16 extra channels. The agent reads these channels but posts only in the channel you are in.

If your connection has a work folder on the server, **Advanced** also shows "Can run commands in {folder}" or "Can read {folder}". You set the folder in **Connection settings…** under **Remote work folder**, and the switch **Let my agent run commands in this folder** decides which line you see. See [Connections and troubleshooting](connections-and-troubleshooting.md).

### Files named in the task

Crew looks for file names in the task: words that end in .csv, .tsv, .xls, .xlsx, .json, .txt, .h5ad or .parquet, in upper or lower case. A name with spaces counts only when it is alone inside double quotes, as in "Plate Reader.csv". Names inside web links are ignored. Crew judges this only after it has loaded every message in the channel.

| Note | When it shows | What to do |
|---|---|---|
| "No file named results.csv is shared in #methods. Your agent will say what it used instead." (with several names: "No files named a.csv, b.csv or c.csv are shared in #methods. …") | The task names a file that no message in the channel shares. | Check the spelling, or share the file in the channel first (see [Messages and files](messages-and-files.md)). The start button still works. The agent then says at the start of its reply what it used instead. |
| "2 files named gina-assay.csv are shared in #methods. Your agent will use the newest one, shared yesterday at 1:55 AM." | Two or more different shared files have the name the task gives. This note sits above **Task**. | Nothing, if the newest copy is right. To use an earlier copy, describe that copy in the task, for example who shared it and when. |

### If Start is refused

Crew's reason appears in a red note above the start button. The task is not started.

| Message | What to do |
|---|---|
| "Choose a model." | Choose a model in **Model**. |
| "{model} isn’t approved for {institution}. Choose a model approved for {institution}, or a local model." | Choose a model marked with the workspace's institution, or a local model. |
| "Private workspace blocks public models", "Restricted Crew context cannot be sent to a public model", "Private Crew context cannot be sent to a public model", or "Institution-owned Crew context cannot be sent to a public model" | Choose a Private model. |
| "This provider controls external tools that Crew cannot isolate. Choose a provider using BioRouter's scoped tools." | Choose a provider other than Claude Code or Codex. |
| "Four Crew tasks are already active on this device. Finish or cancel one first." | Wait for a task to end, or stop one. The Agents section of the Crew sidebar lists your running tasks. |
| "Task must contain between 1 and 32768 bytes." | The task is empty or too long. Shorten it, or share long data as a file and name the file in the task. |
| "Select at most 16 additional channels." | Untick some channels under **Also read**. |
| "Refresh the workspace to verify connection privacy before granting agent access." | Wait until the status row reads "Connected", then try again. If the status row reads "Offline" or "Can’t connect", choose **Connect to {workspace}** in the main area first. See [Connect to a workspace](connections-and-troubleshooting.md#connect-to-a-workspace). |
| "Crew workspace policy changed; refresh before granting agent access" or "Crew context channel is unavailable; refresh before granting agent access" | Open the channel menu (choose the channel name at the top of the channel) and choose **Refresh channel**. Then try again. |
| "Confirm this workspace's institution before granting an agent; unlabelled private workspaces allow human collaboration only" | The workspace has no institution yet. Ask your host to set it. See [Hosting a workspace](hosting-a-workspace.md). |
| "Set this private SSH connection's institution before granting an agent" | Set the institution in **Connection settings…**. See [Connections and troubleshooting](connections-and-troubleshooting.md). |

### If Crew cannot tell whether a task started

Sometimes Crew receives a start request but cannot tell whether the task began, for example when the connection drops at that moment. Starting again could run the same task twice. The pane then shows a warning at the top:

- Title: "Inspect the previous task before starting again"
- Text: "The request to #methods in Analysis Lab was received, but Crew can’t tell whether it started. Repeating it could duplicate its effects."

To continue:

1. Choose **Show task in channel**. The pane closes and Crew scrolls to your newest task in this channel. Or choose **Open chat history** to look for the task's conversation in Biorouter's chat history.
2. Check whether the task ran and what it did.
3. Open **Ask my agent** again. Tick **I checked the previous task and its effects.**
4. Check that **Task** holds the task you want.
5. Choose **Start a new task**. Crew sends it as a new request.

While the warning is open, the main start button stays greyed out. Biorouter remembers the warning until you quit it or reload the window.

### Keyboard use in Ask my agent

- Opening the pane puts the cursor in **Task**.
- Escape closes the pane, unless an open list inside it (such as the model list) closes first.
- When the pane closes, the cursor returns to **Ask my agent**, or to the message box.
- Enter in **Task** starts a new line. Use Tab to reach the start button, then press Enter or Space.
- Switching to another channel or workspace closes the pane. Closing the pane clears its error.

## Follow a task in the channel

### The task row

Each task you start gets a row in the channel. It sits under the task's result, or under the "Task:" post until the result arrives. The row shows a robot icon, "Your agent · " and a status word, the first line of the task, and buttons.

| Status word | What it means | Button on the row |
|---|---|---|
| Starting… | Crew is setting up the task. | None |
| Working… | The agent is working. | **Open** |
| Waiting for your approval | The agent asked to do something that needs your answer. | **Review** |
| Stopping… | Crew is stopping the task. | None |
| Stop not confirmed | This computer stopped the task, but the workspace has not confirmed that the task's access is removed. | **Try stopping again** |
| Interrupted | The Biorouter background service restarted while the task was running, or the task's setup did not finish. Open its conversation and check what it did before you start it again. | **Open** |
| Outcome unknown | Crew could not save the task's latest status. Check its conversation and the channel before you try again. | **Open** |
| Done | The task finished and posted its result. | **Open** |
| Couldn’t finish | The task failed. | **Open** |
| Stopped | The task was stopped. | **Open** |

A status Crew does not recognize appears as plain words. While the channel is being verified again, the row's buttons are greyed out.

### Buttons on the task row

| Button | What it does |
|---|---|
| **Open** | Opens the task's own Biorouter conversation. |
| **Review** | Opens the same conversation, where you answer the agent's approval request. Crew itself approves nothing. |
| **Stop** | Asks "Stop your agent?". Shown while the task can still be stopped. |
| **Try stopping again** | Asks Crew to stop the task again, with no question first. |
| ⋯ (More task actions) | **Show in chat history** opens Biorouter's chat history. **Copy error** copies the task's error, when it has one. **Copy for support** > **Copy task ID** copies the ID that support staff may ask for. |

### Stop a task

1. Choose **Stop** on the task's row.
2. A dialog asks "Stop your agent?" and says "It stops working on this task. Anything it already did stays done."
3. Choose **Stop task**, or **Keep running** to cancel.

The row reads "Stopping…", then "Stopped". If it reads "Stop not confirmed", choose **Try stopping again**. You can also stop a task from an [Agent access list](#rows-in-an-agent-access-list). Messages the agent already posted stay in the channel.

### Answer an approval

When the row reads "Waiting for your approval", choose **Review**. The task's conversation opens. Answer the request there. The Agents section of the Crew sidebar marks such a task with a "Needs you" badge.

### Read the result

The agent's final reply is posted in the channel as the task's result, under "Your agent". The agent names the file it used in its first line. If the file the task named was not shared, the agent says so at the start and names what it used instead.

Crew adds one more line at the end of every result. Crew writes this line from what the task really read, not from the agent's words, so it is the record to trust.

| The task read | The last line reads |
|---|---|
| One shared file | ``Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).`` |
| The newest of several shared files with the same name | ``Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina) at 2:20 AM UTC-7.`` |
| Several files | "Sources: " followed by each file, then "and 1 more file" or "and 3 more files" when there are more. |
| An earlier copy of a file that has a newer copy | The file is marked "(earlier copy)", and the line adds ``A newer copy of `gina-assay.csv` was shared and was not read.`` |
| No shared file | "No shared file was read for this result." |

Crew adds the time a file was shared only when several shared files have the same name, to show which copy the task read. The time uses your computer's clock and time zone. The day comes before the time, as in "on Sep 24 at 2:20 AM UTC-7", when the file was not shared on the day of the result. When Crew cannot place that time, the file is marked "(newest copy)" instead. If Crew does not know who shared a file, the line leaves the person out.

Two other lines can appear:

- "Task finished without a text result." when the agent gave no reply.
- "(This reply was shortened to fit the channel.)" before the last line, when the reply was too long for one message.

### The task's conversation

Each task runs in its own Biorouter conversation. Its title is "Crew · #methods · " followed by the first line of the task, cut to 60 characters. Its first message reads "Crew task for #methods in Analysis Lab", then the task, then "Post the result in #methods in Analysis Lab." If you chose extra channels, it adds "You may read" and their names.

- Open it with **Open** or **Review** on the task's row.
- A new task's conversation may not appear under **Recents** in the Biorouter sidebar. Use **Open** on the row, or **Show in chat history** in the row's ⋯ menu.
- A task can take at most 20 turns and 60 tool calls (actions such as reading a message or a file).
- The agent can use only Crew tools and the checklist tool. The checklist tool is the task list the agent keeps for itself while it works. Its rows in the conversation read, for example, "Adding 3 tasks", "Starting “Plot the data”" or "Marking “Plot the data” complete". See [What a connected chat can and cannot do](#what-a-connected-chat-can-and-cannot-do); the same limits apply.

### When a task ends, its access ends

When a task finishes, fails or is stopped, Crew ends its access right after the result is posted. In the Agent access lists its row then reads "Ended". This is normal. Nobody revoked it and nothing went wrong.

The task's conversation shows "This task is finished. Its access to #methods ended when it finished." with one button, **Start a new chat**. Its send button is greyed out, and the empty message box reads "Start a new chat to continue". Pressing Enter shows "Can’t send" with "This task is finished, and its access to #methods ended with it. Start a new chat to continue." A task's conversation cannot be given access again. To ask for more, start a new task.

## Connect an ordinary chat with /crew

A connected chat lets you keep working in a normal Biorouter conversation while the agent reads a channel and posts in it.

### What a chat needs first

- The chat has at least one message. A new chat, or the Home screen, has nothing to connect yet.
- The message box holds no attached files, images or reference chips. Reference chips are the small labels above the text in the message box. You add one when you type @ and pick a skill, an extension or a knowledge base, or when you right click selected text and choose **Quote it**.
- The chat has a model chosen.
- The chat is not in the middle of a reply.
- The chat is saved on this computer. Every chat you start in the desktop app is saved. This matters only for a command line run started with `--no-session`, or for a chat you deleted.
- If the chat was connected before, you can connect it again only to the same channel in the same workspace, with the same model. See [A chat keeps its first channel and model](#a-chat-keeps-its-first-channel-and-model).

### If /crew shows a toast instead

| Toast | Why | What to do |
|---|---|---|
| "Draft kept" | The message box holds attached files, images or reference chips (the labels you add with @ or **Quote it**). Nothing was sent. | Remove them, then type `/crew` again. |
| "Start the chat first" | The chat has no messages yet. | Send the chat a message, then type `/crew`. To open Crew without connecting a chat, use Crew in the sidebar. |

### Connect a chat

1. Open the chat in Biorouter.
2. In the message box, type `/crew` and nothing else. The command menu shows `/crew` with "Connect this chat to a Crew channel".
3. Press Enter, choose the send button, or pick `/crew` from the menu. Nothing is sent to the model, and nothing is granted yet.
4. Crew opens on the first workspace in its list. With two or more workspaces, that is the top one under "Switch workspace" in the workspace menu. Crew shows the channel you last had open in that workspace, or its first channel if you have not opened one there. For a chat with no access yet, the Chat access pane opens on the right side.
   If the main area reads "{workspace} is offline" instead, choose **Connect to {workspace}**. The Chat access pane opens once Crew has connected and verified the workspace. See [Connect to a workspace](connections-and-troubleshooting.md#connect-to-a-workspace).
5. Check the workspace and the channel the pane names. The pane always asks about the channel Crew is showing. If it is the wrong channel, see [Connect to a different channel](#connect-to-a-different-channel). If the chat belongs in another workspace, see [Connect to a channel in another workspace](#connect-to-a-channel-in-another-workspace).
6. Read what the chat will be allowed to do, for example:
   - "“Plot review” will be able to"
   - "Read #methods"
   - "Post in #methods as Alice Chen (@crew_alice)"
   - "Access ends when you revoke it, or after an hour."
7. If the chat needs other channels, open **Advanced** and tick them under **Also read**. Each reads "Team / #channel". Closed, **Advanced** shows "Reads only #methods" or "Also reads 2 channels".
8. Choose **Allow “Plot review” to read and post in #methods**. When Crew does not know the chat's title, the button reads **Allow this conversation to read and post here**.
9. The pane shows "Connected." and a green "Active" badge. The badge changes to "Active · ends 4:40 PM" once Crew has the end time.
10. Choose **Back to chat**.
11. In the chat, ask for what you need, for example "Read today's messages in #methods and post a short summary there."

Crew does not return you to the chat by itself, so that you can see where **Revoke access** is.

When the consent appears, the cursor is on the Allow button. If you are still holding Enter from step 3, the held key does not press it. Press Enter or Space again, or choose it.

### Connect to a different channel

These steps work only for a chat that has never been connected to Crew. A chat that was connected before keeps its first channel. For a chat that is connected, the note above the message box reads "“Plot review” already uses #methods." instead. See [A chat keeps its first channel and model](#a-chat-keeps-its-first-channel-and-model).

1. In the Crew sidebar, choose the channel you want. Switching channel closes the pane.
2. Above the message box, a note reads "Connect “Plot review” to #imaging?".
3. Choose **Review access**. The Chat access pane opens for that channel.
4. Continue from step 6 above.

A chat holds access to one channel, plus its **Also read** channels.

### Connect to a channel in another workspace

These steps work only for a chat that has never been connected to Crew. A chat that was connected before keeps its first workspace.

1. Choose the workspace name at the top of the Crew sidebar. The workspace menu opens.
2. Under "Switch workspace", choose the workspace. The list appears only when you have two or more workspaces.
3. If the main area reads "{workspace} is offline", choose **Connect to {workspace}** and wait until the status row reads "Connected".
4. In the Crew sidebar, choose the channel.
5. If the Chat access pane does not open by itself, a note above the message box reads "Connect “Plot review” to #imaging?". Choose **Review access**. The Chat access pane opens for that channel.
6. Continue from step 6 of [Connect a chat](#connect-a-chat).

### If Allow is refused

Crew's reason appears in a red note above the Allow button.

| Message | What to do |
|---|---|
| "Wait for the conversation's current turn to finish before granting Crew access." | Let the chat's reply finish, then choose Allow again. |
| "Open the conversation before granting Crew access." | Open the chat in Biorouter, then type `/crew` in it again. |
| "Choose a model before granting Crew access." | Choose a model for the chat. See [Change a chat's model](#change-a-chats-model). |
| "Crew can only grant access to a chat saved on this device. Start a saved chat, then grant it access from Crew." | Start a new chat in Biorouter, send it a message, then type `/crew`. |
| "This conversation contains another institution's context; start a fresh conversation for this workspace" | Start a new chat for this workspace. |
| "An existing Crew conversation retains its original connection, destination and model boundary; start a fresh conversation for another boundary" | The chat was connected before, to another channel or another workspace. Start a new chat, send it a message, then type `/crew` in it. See [A chat keeps its first channel and model](#a-chat-keeps-its-first-channel-and-model). |
| "Crew conversation remains bound to its original resolved provider; start a fresh conversation for another model boundary" | The chat's model no longer matches the model it was first connected with. Start a new chat. |
| "Crew cannot admit providers with external tools outside its scoped capability boundary" | The chat uses Claude Code or Codex. Change the chat's model to another provider, then try again. See [Change a chat's model](#change-a-chats-model). |
| A message about public models, such as "Private workspace blocks public models" | The chat uses a Public model and the channel needs a Private one. Change the chat's model, then try again. See [Change a chat's model](#change-a-chats-model) and [Which models Crew accepts](#which-models-crew-accepts). |
| An institution message | Change the chat's model to one approved for the workspace's institution, or to a local model. See [Change a chat's model](#change-a-chats-model). |
| "Confirm this workspace's institution before granting an agent; unlabelled private workspaces allow human collaboration only" | The workspace has no institution yet. Ask your host to set it. See [What a Private workspace or connection also needs](#what-a-private-workspace-or-connection-also-needs). |
| "Set this private SSH connection's institution before granting an agent" | Set the institution in **Connection settings…**. See [Connections and troubleshooting](connections-and-troubleshooting.md#connection-settings). |
| "Refresh the workspace to verify connection privacy before granting agent access." | Wait until the status row reads "Connected", then try again. If the status row reads "Offline" or "Can’t connect", choose **Connect to {workspace}** in the main area first. See [Connect to a workspace](connections-and-troubleshooting.md#connect-to-a-workspace). |

Changing the chat's model helps only for a chat that has never been connected to Crew. If the chat was connected before, start a new chat instead.

### Change a chat's model

These steps work only for a chat that has never been connected to Crew. A chat that was connected before keeps its first model. See [A chat keeps its first channel and model](#a-chat-keeps-its-first-channel-and-model).

The chat's model control sits in the row under its message box. It shows a brain icon and the model's name. Point at it to see the model's privacy, for example "Private model · UCSF", "Private model · On this machine" or "Public model".

1. Choose the model name under the message box.
2. Choose **Change model**.
3. Pick a Private model that is approved for the workspace's institution, or a local model.
4. Type `/crew` in the chat again, and choose the Allow button.

### While a chat is connected

Above the chat's message box you see:

- A chip that reads "Crew · #methods". Point at it to see "This chat can read and post in #methods." Choose it to open the chat's Chat access pane in Crew.
- A **Revoke access** button. See [Revoke access](#revoke-access).

In the chat's extensions menu (the button under the message box whose tooltip reads "Manage extensions"), the Crew row is switched on and cannot be switched off there. It reads "On while this chat has Crew access. Revoke access to turn it off."

In the channel, you see the chat's posts under "Your agent · Plot review" with an "Agent" badge. Choose the chat title to open that chat. Other people see your name with "'s agent" added, never the chat's title.

If Crew no longer knows the channel's name, the chip names the place another way: "a channel in {workspace}", or "a Crew channel".

### What a connected chat can and cannot do

A connected chat can:

- Read the channel and each **Also read** channel. It can see up to 200 recent messages at once, search the channel, and read files shared there.
- Post in the channel, as your agent.
- Work with files in the connection's **Remote work folder** on the server, when the model is Private. It can run commands there only when **Let my agent run commands in this folder** is on. A Public model can never use the work folder.

A connected chat cannot:

- Use any Biorouter tool other than the Crew tools and the checklist tool (the task list the agent keeps while it works). This means no shell, no web tools and no files on your computer.
- Post in any other channel.
- Give itself access, or revoke its own access. If you ask it to revoke, it tells you that access is not revoked and how to revoke it.
- Act as another person, or change memberships or privacy.
- Move to another channel, another workspace or another model, or be copied or branched. See [A chat keeps its first channel and model](#a-chat-keeps-its-first-channel-and-model).

The agent is told to treat other people's messages and files as information, never as instructions.

These limits stay on the chat permanently, even after its access ends. For other work, start a new chat.

The chat shows one row for each Crew tool the agent uses. To see what was posted, look in the channel.

A chat with no access that asks to use Crew gets this answer: "This chat isn't connected to a Crew channel. To connect it, type /crew in this chat."

### A chat keeps its first channel and model

The first time you allow a chat, Crew records its workspace, its channel and its model. The chat keeps all three permanently, even after you revoke its access or the access expires.

- You can grant it again only to the same channel in the same workspace, with the same model.
- A new grant also keeps every channel the chat could read before under **Also read**.
- You cannot switch the chat to another model.
- You cannot copy or branch the chat, for example with `/diverge`.

| What you try | What Crew or Biorouter says |
|---|---|
| Allow the chat in another channel or another workspace | "An existing Crew conversation retains its original connection, destination and model boundary; start a fresh conversation for another boundary" |
| Change the chat's model | The switch is refused, with an error that contains "Crew conversation remains bound to its original resolved provider; start a fresh conversation for another model boundary". The chat keeps its model. |
| Copy or branch the chat | "Crew conversations cannot be copied or diverged; start a new conversation and grant the desired Crew context." |

In each case, start a new chat, send it a message, then type `/crew` in it. The new chat can use another channel, workspace or model.

### When Crew goes offline

The chat notices within about 15 seconds, and again when you return to it. Above the message box a note appears:

| Note | Button | What it means |
|---|---|---|
| "Crew is offline. It will reconnect by itself when the network is back." | **Connect now** | The network dropped. The button opens Crew and connects the workspace now, on this chat's channel with its access pane. You do not need it: Crew reconnects by itself. |
| "Crew is offline. This chat can’t read or post in #methods until you connect." | **Connect in Crew** | Crew needs you, for example to sign in again. The button opens Crew on the chat's channel with its access pane. Crew then connects the workspace, or shows the screen for the problem, such as signing in again. |

Both buttons take you out of the chat and into Crew. **Revoke access** stays available next to the button. It works without a connection. See [When a revoke says "Stopped on this device"](#when-a-revoke-says-stopped-on-this-device).

You can still type and send while Crew is offline. A reply that needs Crew fails with an error titled "Crew is offline". See [Connections and troubleshooting](connections-and-troubleshooting.md).

### Type /crew in a chat that already has access

Crew opens with a note above the message box and does not open the pane. The note tells you the chat's state:

| Note | Button |
|---|---|
| "Checking this chat’s access…" | None |
| "“Plot review” can read and post in #methods." | **Manage access** |
| "“Plot review” already uses #imaging." | **Manage access** |
| "“Plot review” already uses a channel in {workspace}." | **Manage access** |
| "Crew access for “Plot review” was revoked." | **Grant again** |
| "Crew access for “Plot review” expired." | **Grant again** |
| "Crew settings changed since “Plot review” was given access." | **Grant again** |
| "This task is finished. Its access ended when it finished." | None |
| "Couldn’t load which chats have access." | **Retry** |

When Crew does not know the chat's title, the note says "This chat" instead. Every button opens the Chat access pane. None of them grants or revokes by itself. The note hides while that pane is open.

## See which agents have access

Every list shows only your own chats and tasks on this computer.

| Place | How to get there | What it shows |
|---|---|---|
| **Agent access** tab of the channel details | Channel menu > **Agent access**, the channel header chip, or the details pane's tabs (**About**, **Members**, **Files**, **Agent access**) | Your chats and tasks that can read or post in this channel |
| **Agent access** in Workspace settings | Workspace menu (choose the workspace name at the top of the Crew sidebar) > **Agent access…**, or the **Agent access** tab of Workspace settings | Your chats and tasks in the whole workspace |
| Agents section of the Crew sidebar | Always in the sidebar when something is active | Your running tasks and your connected chats |
| Channel header chip | Top of the channel, for example "2 chats" | How many of your chats and tasks can post here now |
| Chat access pane | `/crew` in a chat, the chat's chip, or a chat row in the Agents section | One chat |
| The chat's own bar | Above the message box in the chat | That chat |

**Agent access…** in the workspace menu stays greyed out until the workspace is connected and verified.

### Rows in an Agent access list

Each row shows:

- A chat bubble icon for a chat, or a robot icon for a task.
- The chat's title, or "Untitled chat". A task reads "Your task" followed by when it started and its first words, for example "Your task · 1:16 PM · Please work out…".
- The channel it posts in, for example "#methods" (or "Team / #methods" when two teams use that name). "+2" means it can read two more channels. "a channel you can’t see" means the channel is not visible to you.
- A status badge.
- Buttons.

| Badge | Meaning |
|---|---|
| "Active · ends 4:40 PM" (green) | It can read and post until 4:40 PM, unless you end it first. Another day shows the date too. |
| "Active" (green) | It can read and post. Crew does not have the end time yet. |
| "Stopped on this device" (amber) | You revoked it. This computer has stopped it. The workspace has not confirmed yet. |
| "Revoked · 2:05 PM" or "Revoked" | You revoked it, and the workspace confirmed. |
| "Expired" | Its hour ran out. |
| "Ended" | A task's access that ended with the task. This is normal. |
| "Ended: Crew settings changed" | The workspace ended it because Crew settings changed after you allowed it. |

| Button | Shown on | What it does |
|---|---|---|
| **Open** | Every row | Opens the chat, or the task's conversation. |
| **Revoke** | An active chat | Asks first, then revokes. See [Revoke access](#revoke-access). |
| **Retry** | A row that reads "Stopped on this device" | Asks the workspace to confirm now. |
| **Stop** | A task that can still be stopped | Asks "Stop your agent?" under the row, with **Stop task** and **Keep running**. |

Rows that read "Stopped on this device" come first, then active rows with the newest first. Revoked, expired and ended rows are under [Past access](#past-access).

When there is nothing to list, the tab says "None of your chats can post in #methods yet." and "To connect one, open that chat and type /crew." (In Workspace settings: "None of your chats can post in {workspace} yet.")

While the list loads it says "Loading agent access…". If it cannot load, a note says "Couldn’t load which chats have access." with **Retry**. If it says "This feature needs a newer Biorouter background service. Quit and reopen Biorouter.", do that.

### The Agents section

The Agents section appears in the Crew sidebar when you have a task that has not finished, or a chat whose access is active or waiting for the workspace to confirm a revoke. It is hidden otherwise.

- A task row shows a status dot and "#methods · Working…". It carries a "Needs you" badge when the task waits for your approval. Choose it to go to the task's channel and highlight the task.
- A chat row shows a chat icon and "Plot review · #methods". It carries a badge only when the access is not active. Choose it to open that chat's Chat access pane. This works even while the workspace is offline.
- To see finished tasks and revoked chats too, choose ⋯ (**Agents options**) next to the heading and tick **Show revoked and finished**.

The rows have no revoke button. Revoke from the pane that a chat row opens.

### The channel header chip

The chip appears at the top of a channel when at least one of your chats or tasks can post there now. It reads "1 chat", "2 chats", "1 task", "2 tasks", or "3 agents" when there are both chats and tasks. Choose it to open the channel's **Agent access** tab.

## Revoke access

### What a revoke does

- This computer stops the chat from reading or posting at once, and saves that. Then Crew asks the workspace to confirm.
- The whole chat stops. You cannot send more messages in it until you grant access again.
- Messages the agent already posted stay in the channel.
- If the agent started a command in the server's work folder, a revoke does not prove that the command has stopped.
- Deleting a chat does not revoke its access. Revoke first. A deleted chat's access stays in the Agent access lists until you revoke it or it expires.
- Task rows have no **Revoke** button. To end a task's access, stop the task.

Every revoke asks first, in the same words:

- "Stop “Plot review” reading and posting in #methods?"
- "This chat will stop until you grant access again."
- Buttons **Revoke** and **Keep access**.

The cursor starts on **Keep access**. Escape means **Keep access**.

### Revoke from the chat

1. In the chat, choose **Revoke access** next to the "Crew · #methods" chip.
2. Choose **Revoke**.

The bar above the message box then changes to the revoked message. See [After access ends](#after-access-ends).

### Revoke from the Chat access pane

1. Open the pane: choose the chip in the chat, type `/crew` in the chat and choose **Manage access**, or choose the chat's row in the Agents section.
2. Choose **Revoke access**.
3. Choose **Revoke**.

The pane shows "Access revoked. “Plot review” can’t use Crew until you grant access again." with **Open chat** and **Done**. **Done** closes the pane.

### Revoke from an Agent access list

1. Open the channel's **Agent access** tab, or **Agent access** in Workspace settings.
2. Choose **Revoke** on the chat's row.
3. Choose **Revoke** under the row.

The result appears once, above the list. Choose **Done** to clear it.

### Revoke from the command line

Run `biorouter crew grants revoke <SESSION>`, where `<SESSION>` is the chat's session ID. It exits with status 0 only when the workspace confirmed the revoke. See [Command line](command-line.md).

To find the session ID, run `biorouter crew grants list`. Each row shows it after the chat's title, for example `Chat "Plot review" (chat 20260924_2)`. `biorouter session list` also prints every chat's ID, name and last update.

### When a revoke says "Stopped on this device"

This means the workspace could not be reached when you revoked. This computer has already stopped the chat. It cannot read or post from here. The workspace has not confirmed yet.

- While the connection is down, the note reads "Stopped on this device. Crew confirms it with the workspace when it reconnects."
- When the connection is back, it reads "Stopped on this device. Confirming with the workspace…"
- Lists and the pane show the amber badge "Stopped on this device".

You do not need to do anything. Crew asks the workspace again each time it connects, and keeps asking while connected until the workspace confirms. It remembers this across restarts. **Retry** asks at once, but you do not need it.

When the workspace confirms, the note reads "Confirmed. The workspace has stopped this chat’s access too." It stays until you choose **Dismiss** or leave that screen. The row then reads "Revoked · {time}".

### When a revoke says "Not revoked"

The note reads "Not revoked. This chat can still read and post." followed by Crew's reason, with **Retry**.

| Reason | What to do |
|---|---|
| "No Crew grant for this session." | The chat has no access to revoke. Check the Agent access lists. |
| "This session's Crew grant belongs to a different Crew connection." | Open the workspace the chat is connected to, then revoke from there. |
| "This session was granted Crew access again while its previous grant was being revoked. The new grant is active; revoke again to stop it." | Revoke again. |

## After access ends

A chat's access ends in one of three ways. Each looks different in the chat.

| Cause | Note above the message box | "Can’t send" toast when you press Enter |
|---|---|---|
| You revoked it | "Crew access to #methods was removed, so this chat can’t continue. It holds messages from the channel. Grant access again to continue, or start a new chat." | "Crew access to #methods was removed. Grant it again or start a new chat." |
| One hour passed | "Crew access to #methods expired. This chat has team content, so it can’t continue." (Team content means messages from the channel.) | "Crew access to #methods expired. Grant it again or start a new chat." |
| Crew settings changed | "Crew settings changed since this chat was given access to #methods, so it can’t continue. Grant access again to continue, or start a new chat." | "Crew settings changed since this chat was given access to #methods. Grant it again or start a new chat." |

In each case:

- The note offers **Start a new chat** and **Grant access again**.
- The send button is greyed out, and the empty message box reads "Grant access again to continue this chat".
- Editing a message, retrying, sending again and queued messages are held the same way.
- If a reply was running when access ended, the chat shows an error with no retry button, titled "Crew access removed" or "Crew access ended". Its text is one of these: "This chat's Crew access was removed. Start a new chat, or grant access again from Crew.", "Crew settings changed since access was granted. Grant access again from Crew.", "This chat's Crew access has ended. Grant access again from Crew to continue.", or "This chat's Crew access is no longer available. Start a new chat, or grant access again from Crew."
- A revoke that the workspace has not confirmed shows its "Stopped on this device" note above.

The chat keeps the messages it read from the channel, and it stays limited to Crew tools and the checklist tool. That is why it cannot continue as an ordinary chat.

### Grant access again

1. In the chat, choose **Grant access again**.
   Crew opens on the chat's workspace and channel, with the Chat access pane showing the consent.
2. Review what the chat will be able to do.
3. Choose the Allow button.

This gives the chat new access for up to one hour.

In Crew, the note's **Grant again** button opens the same consent, but only for the chat you arrived with through `/crew` or the chat's own buttons. If you opened a revoked chat from the Agents section, the pane says "To connect it again, type /crew in that chat." with **Open chat**.

### Start a new chat instead

Choose **Start a new chat**. The new chat has no Crew access and none of the old chat's limits. The old chat stays in your chat history.

### Access that ended because Crew settings changed

Access is tied to the settings that were in place when you allowed it. It ends when:

- Your connection's privacy changes, when its connection settings are saved with a change, or when its institution changes.
- The workspace's privacy settings change, or its memberships change (for example, someone is removed from a channel).
- What the chat reads becomes protected (for example, a channel it reads is marked Restricted), or the institution changes.

Crew warns before some of these changes. For example, the dialog that makes a workspace Private says "Agents with access will need permission again." Removing a workspace from this computer says "Chats connected to it lose access."

A change on your connection shows in the chat within seconds. A change on the workspace shows the next time the chat uses Crew, so an idle chat can still look active until then. The list badge reads "Ended: Crew settings changed". To continue, grant access again or start a new chat.

### Access that expired

Access ends one hour after you allow it, even with no network. The list badge reads "Expired", and the Crew note reads "Crew access for “Plot review” expired." with **Grant again**.

## Past access

Revoked, expired and ended rows are folded under **Show past access (3)** below each Agent access list. Choose it to open the list, which is named "Past access". When only past rows exist, the empty sentence appears above it.

Biorouter keeps one access record per chat, so a chat you grant again would lose its earlier revoked row. To keep those rows, this computer remembers each revoke it saw confirmed, with the time, for example "Revoked · 2:05 PM". It keeps up to 100 rows per workspace connection. These rows are for display only. On another computer, or after Biorouter's stored data is cleared, they do not appear.

In the Agents section, choose ⋯ (**Agents options**) and tick **Show revoked and finished** to see the same kind of rows there.

## Command line

People who use a terminal can do the same things with `biorouter crew`. Full details are in [Command line](command-line.md).

| What you want | Command |
|---|---|
| List your chats' and tasks' access | `biorouter crew grants list` |
| Connect a chat to a channel | `biorouter crew grants grant <SESSION> <CHANNEL>`, with `--context-channel` for each extra channel |
| Revoke a chat's or a task's access | `biorouter crew grants revoke <SESSION>` |
| Start a task | `biorouter crew tasks start --provider <PROVIDER> --model <MODEL> --allow-posting --text "<TASK>" <CHANNEL>` |
| List your tasks | `biorouter crew tasks list` (add `--show-ids` to see task IDs) |
| Stop a task | `biorouter crew tasks cancel <RUN>` |

`<SESSION>` is a chat's session ID, such as `20260924_2`. `grants list` shows it in each row, and `biorouter session list` prints every chat's ID. `<CHANNEL>` can be written `methods`, `'#methods'` or `analysis-lab/methods`. In `grants list`, a revoke waiting for the workspace reads "Stopped on this device; the workspace hasn't confirmed yet", and a grant the workspace ended reads "Ended: Crew settings changed".

## Common problems

| What you see | Why | What to do |
|---|---|---|
| No **Ask my agent** button | The channel is archived ("This channel is archived."), or Crew is still checking it ("Verifying access…"). | Wait for the check to finish, or use a channel that is not archived. |
| The start button stays grey | See the list under [What the pane shows](#what-the-pane-shows). | Read the notes in the pane. |
| The agent used the wrong file | The file was not shared in the channel, or several files have that name. | Read the task's last line. Share the file, or describe which copy to use, and start a new task. |
| A task row reads "Ended" in Agent access | The task's access ended with the task. | Nothing. This is normal. |
| The task's conversation is not under **Recents** | A known gap in the Biorouter sidebar. | Use **Open** on the task row, or **Show in chat history**. |
| `/crew` shows "Start the chat first" | The chat has no messages. | Send it a message first. |
| `/crew` shows "Draft kept" | Files, images or reference chips are attached. | Remove them first. |
| The Chat access pane names the wrong channel | The pane always asks about the channel Crew is showing. | Choose the right channel, then **Review access**. |
| The chat answers "This chat isn't connected to a Crew channel. To connect it, type /crew in this chat." | The chat has no access. | Type `/crew` in the chat. |
| The chat's send button is grey | The chat's access ended. | Choose **Grant access again**, or **Start a new chat**. |
| Allow is refused with "An existing Crew conversation retains its original connection, destination and model boundary; start a fresh conversation for another boundary" | The chat was connected before, to another channel or workspace. | Start a new chat for the new channel. See [A chat keeps its first channel and model](#a-chat-keeps-its-first-channel-and-model). |
| A chat can no longer use a tool it used before | A chat that was connected to Crew keeps only Crew and checklist tools permanently. | Start a new chat for that work. |
| An Agent access list is empty, but someone's agent posts in the channel | The lists show only your own chats and tasks. | Ask that person. |
| A revoke says "Stopped on this device" | The workspace could not be reached. The chat is already stopped here. | Nothing. Crew confirms by itself when the connection is back. |

## Related documentation

- [Crew user manual](README.md): the index of all Crew pages.
- [Getting started](getting-started.md): how to open Crew and the parts of the Crew window.
- [Messages and files](messages-and-files.md): how to share the files a task reads.
- [Teams, channels and people](teams-channels-and-people.md): channels, channel menus and archiving.
- [Privacy and security](privacy-and-security.md): Private, Public, Restricted channels and institutions, which decide which models an agent may use.
- [Connections and troubleshooting](connections-and-troubleshooting.md): what to do when Crew is offline or a connection fails.
- [Command line](command-line.md): the `biorouter crew tasks` and `biorouter crew grants` commands in full.
- [Administration](administration.md): server setup, where Crew keeps its data, and the limits it applies.
