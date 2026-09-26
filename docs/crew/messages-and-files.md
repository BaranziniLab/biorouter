# Messages and files

> **What this is.** How to write, send, read and copy messages in a Crew channel, how Crew keeps
> your unsent drafts, and how to share files from your computer or from the lab server.
> **Status:** Current.
> **Audience:** Crew members who use the Biorouter desktop app. No terminal experience is needed,
> except for the short command line section near the end.

Every channel has a message list in the middle of the window and a message box at the bottom. This
page covers everything you do in those two places, and the **Files** tab where a channel's files are
listed. It assumes you have joined a workspace and can see at least one channel in the sidebar. If
you cannot, start with [Joining a workspace](joining-a-workspace.md).

## Parts of the channel screen

| Part | Where it is | What it holds |
|---|---|---|
| Channel header | Top of the middle column | The channel name (choose it to open the channel menu), the "Restricted" or "Public-safe" badge, the member avatars, and the **Channel details** button. |
| Message list | Middle column | The channel's messages, oldest at the top and newest at the bottom. |
| Message box | Bottom of the middle column | The attachment chips, the space where you type, the paperclip button (tooltip "Attach"), **Ask my agent**, and the round Send button. |
| Details pane | Right side, when you open it | The tabs **About**, **Members**, **Files** and **Agent access**. |

**Ask my agent** opens and closes the agent pane on the right side. It is covered in
[Agents and chat access](agents-and-chat-access.md).

Only one note shows above the message box at a time. A note about your last action, such as a
failed send or a refused file, comes before any other note.

## Write and send a message

### Send a message

1. Select the message box at the bottom of the channel. Until you type, it reads "Message #methods",
   with your channel's name in place of `methods`.
2. Type your message. To start a new line without sending, press Shift+Enter.
3. Press Enter, or choose the round Send button at the right end of the message box. Its tooltip reads
   "Send message".

The Send button stays grey until the message has text, a finished file or a server path. After you
choose it, the cursor returns to the message box so you can keep typing.

The message box starts one line tall. It grows as you type, up to about 40% of the window height, and
then scrolls.

### Keys in the message box

| Key | What it does |
|---|---|
| Enter | Sends the message. |
| Shift+Enter | Starts a new line. |
| Enter while you type with a Japanese, Chinese or Korean input method | Confirms the characters. It does not send. |
| Holding Enter down | Sends once. The repeated key presses are ignored. |

### What happens after you press Send

1. Your text stays in the message box until the workspace accepts the message. During that time you cannot
   change the text, and the Send button shows a spinner (tooltip "Sending…"). Pressing Enter or Send
   again does nothing until the first attempt finishes.
2. When the workspace accepts it, the message box empties and the message appears at the bottom of the
   list under your name. It is dimmed and shows a spinner and "Sending…".
3. When the update arrives from the workspace, the message turns into a normal message. If it is
   the first message of the day, it appears under a "Today" band.

Sending also does three more things:

- It marks the channel as read up to your message, so your own reply never sits under the "New"
  line.
- If you were reading earlier messages, it takes you back to the newest messages.
- It removes any kept draft for that channel (see [Drafts you leave behind](#drafts-you-leave-behind)).

### If a message does not send

- A red note above the message box starts with "Couldn’t send." and then gives the reason.
- Your text and files stay in the message box.
- Fix the cause and press Send again. If you have not changed the message, Crew repeats the same
  request, so the message cannot be posted twice.

If the note reads "Refresh the workspace to verify connection privacy before sending.", Crew has
not finished checking the connection. Wait until the status row under the workspace name in the
sidebar reads "Connected", then press Send again. If it does not reach "Connected", see
[Connections and troubleshooting](connections-and-troubleshooting.md).

### Limits on messages

| Limit | Value |
|---|---|
| Length of one message | 65,536 bytes of text (64 KB). English letters and digits take one byte each; accented letters, other scripts and emoji take more. |
| Messages in one workspace | 100,000 in total. |

A message over either limit is refused with the same "Couldn’t send." note above the message box. To
share a long log or table, save it as a file and attach the file instead.

### What messages cannot do

- A posted message cannot be edited or deleted. Read your message before you press Send.
- There are no threads, reactions or pinned messages.
- Typing `@bob` does not notify Bob. It stays plain text.
- The desktop app has no message search. The command line has one: see
  [Messages and files from the command line](#messages-and-files-from-the-command-line).
- You always post as yourself. Crew has no way to post under another person's name.

## Format a message

Crew reads your message as Markdown, a plain text way of marking formatting. The message box shows
the characters exactly as you type them. The formatting appears only in the posted message. There is no
formatting toolbar and no preview.

| You type | Readers see |
|---|---|
| `**important**` | **important** in bold |
| `*note*` | *note* in italics |
| `~~old value~~` | the words struck through |
| `` `od600` `` | `od600` in a code font |
| A line starting with `- ` or `1. ` | A bulleted or numbered list |
| `- [ ] check pH` | A checkbox. Readers cannot tick it. |
| A line starting with `> ` | A quotation |
| `---` on its own line | A horizontal line |
| A table drawn with vertical bars between the columns | A table |
| `# Results` | A bold line. Crew does not make large headings. |
| `https://example.org` | A link that opens in your web browser. Hover over it to see the full address. |
| `![gel image](https://example.org/gel.png)` | A link labeled "Image: gel image". Crew never loads the picture. |

More rules:

- A single Shift+Enter in the message box stays a line break in the posted message.
- Only addresses that start with `http`, `https` or `mailto` become links. Anything else, including
  a path to a file on your computer, shows as plain text.
- HTML tags show as the characters you typed. Math notation is not rendered.
- A block fenced with three backticks (```` ``` ````) becomes a code box. Its top line shows the
  language you named after the backticks, or "code" if you named none, and a **Copy code** button.
  Code is not colored and cannot be run from Crew.
- A table or code box wider than the column scrolls sideways inside its own box. A fade on the right
  edge shows there is more.

## Read a channel

### How messages are grouped

- A channel opens at its newest message. While you are at the bottom, new messages scroll into
  view.
- Messages the same author sends in a row form one group. The group starts with a head row: the
  avatar (a circle for a person, a square robot tile for an agent), the author's name, and the time,
  such as "10:02 AM".
- A new group starts when someone else posts, after a gap of more than five minutes, on a new day,
  at the "New" line, and when a person's messages change to their agent's messages.
- The rows after the head show only the text. Point at one to see its time, such as "10:03", in the
  left margin.
- Hover over the time in a head row to see the full date, such as "Tuesday, September 22, 2026 at
  10:02 AM". Times use your computer's time zone.

### Day bands

A band across the list marks each day: "Today", "Yesterday", a weekday and date such as "Tuesday,
September 22" for this year, or a full date such as "September 22, 2025" for an earlier year. The
band stays at the top of the list while you scroll through that day. At midnight "Today" changes to
"Yesterday" by itself.

### Long messages

A message longer than 10 lines or 600 characters is folded to about nine lines, with a fade at the
bottom. Under it, **Show more** opens the whole message and **Show less** folds it again. Beside the
button Crew states the full size, such as "214 lines · 8.4 KB" for a log or table, or "128 words"
for prose.

### Messages from agents

- An agent's messages carry an "Agent" badge. The author reads "Bob Lee's agent", or "Your agent"
  for yours.
- When one of your own chats posted the message, the head reads "Your agent" followed by the chat's
  title. Choose the title to open that chat. Another person's agent is never named by their chat.
- An agent's step by step updates, such as "Using …" or "Tool failed: …", are folded into one row.
  Choose **Show details** to see them. The row states how many there are, such as "3 updates".

For everything else about agents, see [Agents and chat access](agents-and-chat-access.md).

### The Restricted marker on a message

A message whose privacy level differs from its channel's shows a "Restricted" marker after its
time. The tooltip reads "Only private models can read this message." See
[Privacy and security](privacy-and-security.md) for what Restricted means.

### Move through a long channel

- If you scroll up while new messages arrive, a button appears above the message box, such as "3 new
  messages" with a down arrow. It counts messages from other people, not agent updates. Choose it to
  go to the newest message. When nothing has been counted it reads **Jump to latest**.
- Crew loads the newest 200 messages of a channel. When there are more, an **Older messages** row
  sits at the top of the list. Scroll up to it, or choose it, to load the next page. It reads
  "Loading earlier messages…" while it works.
- While you read an older page, a button reads "Viewing earlier messages" with **Jump to latest**.
  New messages are not shown until you choose **Jump to latest**.
- When a channel first opens, "Loading messages…" shows until the messages arrive.

### Use the keyboard in the message list

| Key | What it does |
|---|---|
| Tab | Moves into the message list. The whole list is one Tab stop. |
| Up Arrow and Down Arrow | Move from message to message. |
| Home and End | Go to the first and the last loaded message. |
| Tab, on a message | Reaches that message's buttons and file cards. |

Inside a link or a button, the arrow keys belong to that control. Screen readers announce these keys
when you enter the list.

## Unread messages

### How unread messages are shown

- In the sidebar, a channel with unread messages has a bolder name and a count. The count stops at
  "99+".
- Inside the channel, a thin colored line with "New" at its right end marks the first message you
  have not read. Screen readers call it "New messages".
- When the first unread message is also the first message of its day, the day band itself changes
  color and carries "New", instead of a second line.
- Crew places the "New" line when you open the channel. It stays in the same place until you leave
  the channel, and it disappears once you post there.
- Your own messages never start a "New" line. A channel whose only new messages are yours shows
  none.

### Automatic mark as read

Crew marks a channel as read by itself when all of these are true:

- The Biorouter window is the active window and is visible.
- You are scrolled down to the newest message.
- The newest message has stayed in view for one second.

Crew does this at most once every five seconds for each channel. It never marks a channel as read
while you read older messages. If marking as read fails, Crew says nothing and tries again with the
next new message.

### Mark a channel as read yourself

- Choose the channel name in the channel header to open the channel menu, then choose **Mark as
  read**. It is unavailable while no message is loaded or while you read older messages.
- Or right click the channel in the sidebar and choose **Mark as read**. This item appears only on
  the channel you have open, when it has unread messages and you are viewing its newest messages.
  On the keyboard, Shift+F10 or the Menu key opens the same menu.

## Copy a message

1. Point at the message, or move to it with the arrow keys and press Tab.
2. Choose the copy button beside it. Its tooltip reads "Copy text".
3. For two seconds the button shows a check mark and the tooltip reads "Copied".

The **More actions** button (⋯) beside it opens a menu with **Copy text**, and **Copy for support**
holding **Copy message ID**. You need a message ID only when someone helping you with a problem
asks for it.

- **Copy text** copies what the author typed, including Markdown characters such as `**` and
  backticks, not the formatted result.
- If the button shows an X and "Couldn’t copy", select the text with the mouse and press Cmd+C on
  macOS, or Ctrl+C on Windows and Linux.
- To copy the contents of a code box, use its **Copy code** button.

## Drafts you leave behind

When you leave a channel with text still in the message box, Crew keeps that text. This happens when you
switch to another channel, team or workspace, or leave Crew. When you come back to the channel, the
text is back in the message box.

- A channel that holds a kept draft shows a pencil in the sidebar where the unread count would be.
  If the channel also has unread messages, the count shows instead. Screen readers hear ", draft"
  after the channel name.
- Only the text is kept. Files and server paths in the message are not kept. A file you uploaded but
  did not send waits in the channel's **Files** tab under "Uploaded, not sent". See
  [Put an unsent upload back into a message](#put-an-unsent-upload-back-into-a-message).
- Drafts are kept in memory only. Quitting Biorouter discards them.
- Crew keeps at most 50 drafts. When there are more, the oldest is dropped first. A draft larger
  than 64 KB is not kept at all.
- A draft comes back only into its own channel, only into an empty message box, and only if the
  channel's privacy and your access to it have not changed. If they changed, Crew clears the draft
  and tells you: "Workspace privacy or selected channel access changed, so your unsent draft was
  cleared."
- If you lose access to a channel while it is open, Crew closes it and says, for example, "You no
  longer have access to #methods, so it was closed." If the message box held text, Crew adds "Your
  unsent draft for it was cleared."
- While Crew checks your access to a channel, a bar reading "Verifying access…" replaces the
  message box. Your draft is kept during the check.
- In an archived channel the message box is replaced by "This channel is archived." Nobody can post
  there.

## Share a file from your computer

### What sharing a file does

- The upload to the workspace server starts as soon as you pick the file and confirm. The contents
  leave your computer at that moment.
- Other people see the file only after you press Send. Until then it is part of the message you are
  writing.
- A file joins your message only when its upload has finished. Wait for the upload to finish before
  you press Send. If you send text before then, the text goes alone. If you stay in the channel,
  the file is added to the message box when its upload finishes, ready for your next message.
- You can put several files in one message by adding them one after another. Each upload takes one
  file.
- Taking a file out of your message does not delete the uploaded copy. Crew has no way to delete a
  file once it is uploaded. Check that you picked the right file before you confirm.

There are three ways to add a file: the Attach menu, dragging it into the channel, and pasting it.

### Upload a file with the Attach menu

1. Choose the paperclip button at the lower left of the message box. Its tooltip reads "Attach".
2. Choose **Upload a file…**.
3. Your system's standard file window opens (the Finder panel on macOS). Select one file and choose
   **Open**.
4. A chip with the file name appears in the message box, above your text, and the upload starts.
5. Type a message if you want one. When the upload is finished, the line "Press Send to share it."
   appears under the chips.
6. Press Enter or choose Send.

**Upload a file…** is unavailable while a file window or a Share dialog is already open.

### Drag a file into the channel

1. Drag a file from Finder or File Explorer onto the channel. You can drop it anywhere on the message
   list or the message box, but not on the sidebar or the details pane.
2. While you drag, the list is tinted and reads, for example, "Drop to attach in #methods". Release
   the mouse button.
3. A confirmation dialog opens. Its bold first line reads, for example, 'Share "counts.csv" (55 KB)
   to Crew?'. Check the three lines under it:

   ```text
   Full path: /Users/alice/Data/run42/counts.csv
   Destination: #methods in chen-lab
   It uploads now and appears in #methods when you send your message.
   ```

   The full path is the file's real location. If you dropped a shortcut, the path names the file it
   points to.
4. Choose **Share**. **Cancel** is the default button, so pressing Return or Escape shares nothing.
5. The chip appears in the message box and the upload starts.
6. When the chip is finished, press Enter or choose Send.

While the dialog is open, the message box shows "To share it, choose Share in the dialog." If you
choose **Cancel**, nothing is shared and Crew shows nothing.

You cannot drop files into an archived channel, or while Crew is still checking your access.

### Paste a file

1. Copy a file in Finder or File Explorer.
2. Place the cursor in the message box and paste: Cmd+V on macOS, Ctrl+V on Windows and Linux.
3. The same confirmation dialog opens, starting with 'Share "counts.csv" (55 KB) to Crew?'. Continue
   from step 4 above.

Pasting follows two more rules:

- A pasted screenshot or picture that is not saved as a file is refused with "Crew can share saved
  files only. Save it as a file first, then share it again."
- If the clipboard holds text as well as a picture, for example cells copied from a spreadsheet,
  Crew pastes the text.

### Watch the upload

The chip in the message box shows the upload's progress.

| The chip shows | What it means |
|---|---|
| A spinner and "Uploading…" | The upload has started. Crew shows this for the first second and until 1% has moved. |
| A progress ring and a percentage, such as "42%", with a pause button | The upload is moving. The pause button's tooltip reads "Pause upload". |
| "Starting…", "Finishing…" or "Pausing…" | The upload is between steps. |
| "Paused" with a play button | You paused it. Choose the play button to resume. |
| "Failed" with a play button | It stopped. Hover over the chip to read the reason, then choose the play button to try again. |
| The file name with an × | The upload is finished and the file is in your message. |

A file that finishes in under a second never shows the pause button.

When you resume a paused or failed upload, the file window opens again. Select the same file. Crew
refuses a different file.

If you leave the channel before an upload finishes, the upload carries on. The finished file then
waits in that channel's **Files** tab under "Uploaded, not sent".

### Notes under a file chip

Crew compares a new file with the files already shared in the channel. When it finds a match, it
shows a note under the chip. The notes never stop you from sending.

| Note | What it means |
|---|---|
| "counts.csv is already in #methods (shared 6:54 PM). Remove this one if it’s the same file." | A file with the same name is already in the channel. Crew does not know yet whether the contents match. |
| "counts.csv is the same file as the one shared at 6:54 PM. You can remove it." | The contents are identical to a file already in the channel. |
| "A different counts.csv was shared at 6:54 PM. Agents will use this newer one once you send it." | The name matches but the contents differ, for example a corrected file. After you send it, agents that read counts.csv use the newest copy. |

### Take a file out of your message

Choose the × on the file's chip. Its tooltip reads, for example, "Removes it from this message. The
copy already uploaded stays on labserver.", with your server's name.

The file then appears in the **Files** tab under "Uploaded, not sent", where **Attach** can put it
back.

To take a server path out of your message, choose the × on its chip. Screen readers call that button
"Remove remote reference" followed by the path's label.

### Files Crew will not share

| What you tried | What Crew says |
|---|---|
| More than one file at once | Crew takes the first file and adds "Crew shares one file at a time." |
| A folder | "Crew shares files, not folders. Choose a file inside the folder." Compress the folder into a zip file first, or share the files inside it one by one. |
| A file larger than 1 GB | "counts.csv is larger than 1 GB. Crew can share files up to 1 GB." For large datasets that are already on the lab server, share a server path instead. |
| A picture or screenshot that is not saved as a file | "Crew can share saved files only. Save it as a file first, then share it again." |
| A file that looks like a password, key or token store | "“id_rsa” looks like a credential file (a password, key or token store), so Crew won't share it." |
| A drop or paste while a Share dialog is open | "Finish the open share confirmation first." |

After you choose **Share** in the dialog, Crew checks the file again and can still refuse it:

| Message | What to do |
|---|---|
| "This item isn't a saved file, so it can't be shared. Save it as a file first, then share it again." | Save the item as a file, then share the file. |
| "\"counts.csv\" is no longer there. It may have been moved or deleted." | Find the file and drop it again. |
| "Biorouter can't read \"counts.csv\". Check that you're allowed to open it, then try again." | Check that you can open the file yourself. |
| "\"results\" is a folder. Crew shares one file at a time, so zip the folder or drop the files inside it." | Zip the folder, or drop the files one by one. |
| "\"counts.csv\" is a shortcut to a file in another folder. Drop the original file instead." | Find the original file and drop that. |
| "\"counts.csv\" is a shortcut to something that no longer exists." | Find the original file, if it still exists. |
| "\"counts.csv\" isn't a regular file (it's a device, a socket or a pipe), so it can't be shared." | Save the data to an ordinary file first. |
| "\"counts.csv\" is larger than 1 GB, the most Crew can attach." | Share a server path instead, or split the file. |
| "\"counts.csv\" changed while you were deciding. Drop it again to share the current version." | Drop the file again. |
| "Connection privacy changed. Refresh Crew and drop the file again." | Wait for "Connected" in the status row, then drop the file again. |
| "Crew couldn't take \"counts.csv\". Check that the file is readable, then drop it again." | Check that you can open the file, then drop it again. |

The credential check applies to every way a file reaches Crew: the Attach menu, dragging, pasting
and the command line. Crew refuses private keys, cloud credential files, `secrets` files, `.env` files,
and any file whose first 64 KB contain credential material. The Share dialog
appears first, so this refusal can come after you choose **Share**.

## Open and save a shared file

### The file card

A shared file appears in its message as a card with a file icon, the file name and its size, such as
"55 KB" or "1.5 MB". Hover over the name to see all of it. When two cards in view share a name, each
card also shows when it was posted, such as "6:54 PM".

The card's buttons are always visible. With the keyboard, they become Tab stops once you move to
that message:

| Button | What it does |
|---|---|
| Save (download icon, tooltip "Save counts.csv") | Downloads the file to a place you choose. |
| Preview (eye icon, tooltip "Preview gel.png") | Shows the picture under the card. Only PNG, JPEG, GIF and WebP pictures have this button. Choose it again to hide the preview. |
| **More actions** (⋯) | Opens a menu with **Save counts.csv…**, the download controls described below, and **Copy for support** holding **Copy file ID** and **Copy SHA-256**. |

You need the file ID or the checksum only when someone helping you with a problem asks for it.

While a card's details are still loading, its name reads "Attachment". If they cannot load, the card
reads "Crew couldn’t load this file’s details."

### Save a file

1. Choose the Save button on the card, or choose **More actions** (⋯) and then **Save counts.csv…**.
2. Your system's standard save window opens (the Finder panel on macOS) with the file name filled
   in. Choose a folder, then choose **Save**.
3. A thin bar along the bottom of the card shows progress. Beside the size, the card reads, for
   example, "Downloading 42%". In the **Files** tab the same download reads "Downloading…" until 1%
   has moved.
4. When the download is finished, the bar goes away.

Crew downloads into a temporary file first. It checks the downloaded file against the shared file's
checksum and only then puts it in place under the name you chose. If the check fails, Crew empties
the temporary file, stops, and shows the reason under the card. Choose **Resume…** to download
again.

If a file with the same name already exists in that folder, a dialog opens with the bold line
"Replace counts.csv after the download is verified?". The existing file stays in place until the
download passes its check. Choose **Replace file** to go ahead, or **Cancel**
to keep the existing file.

Crew refuses to save into folders that hold credentials or settings: "Crew won't save into a
credential or settings location. Choose another folder."

While a download is moving, **More actions** (⋯) offers **Pause**. A paused or failed download
offers **Resume…** and **Remove from list** there. See
[Pause, resume and clear transfers](#pause-resume-and-clear-transfers).

If you are removed from the channel, a download you start afterwards fails with "That channel isn't
available to you. It may be archived, or you may not be in it."

## The Files tab

### Open the Files tab

- Choose **Channel details** at the right end of the channel header, then choose the **Files** tab.
- Or choose the channel name in the header to open the channel menu, then choose **Files**.

### The four sections

Each section appears only when it has something in it.

| Section | What it lists | What you can do there |
|---|---|---|
| "In progress" | Uploads and downloads for this channel on this computer that have not finished. Each row shows an upload or download icon, the name, the state such as "Uploading 42%", the size and a progress bar. | **Pause** while it moves. Otherwise **More actions** (⋯) offers **Resume…** and **Remove from list**. |
| "In your message, not sent yet" | Files attached to the message you are writing, with their size. A message on its way shows its files here as "Sending…" until it appears in the channel. | Nothing. To take a file out, use the × on its chip in the message box. |
| "Uploaded, not sent" | Finished uploads to this channel that are in no loaded message and not in the message box. | **Attach** puts the file back into your message. **More actions** (⋯) offers **Remove from list**. |
| "In this channel" | Files and server paths in the loaded messages, newest first. A line above each one names who shared it and when, such as "Bob Lee · 6:54 PM". | The same buttons as in the message: Save, Preview and **More actions** on a file, **Copy** on a server path. |

"In this channel" lists only the files in messages Crew has loaded: the newest 200 messages, plus any
older pages you loaded by scrolling up. To see older files, scroll up in the message list first.

If the tab is empty, it reads "No files in this channel yet." If Crew cannot read this computer's
transfer list, the tab reads "Crew couldn’t check this computer’s transfers."

### Put an unsent upload back into a message

1. Open the channel's **Files** tab.
2. Under "Uploaded, not sent", find the file and choose **Attach**.
3. The file's chip appears in the message box. Press Send to share it.

Crew checks the upload again before it attaches it. If the upload no longer matches the shared file,
you see "This upload no longer matches its shared file, so it wasn’t attached." Upload the file
again instead.

## Pause, resume and clear transfers

A transfer is one upload or one download on this computer. Crew keeps a record of each one.

### Pause a transfer

- An upload in the message box: choose the pause button on its chip.
- A download: choose **More actions** (⋯) on the file card, then **Pause**.
- Either kind: choose **Pause** on its row under "In progress" in the **Files** tab.

The state reads "Pausing…" and then "Paused".

### Resume a transfer

- An upload: choose the play button on its chip, or choose **Resume…** on its row in the **Files**
  tab. In the file window that opens, select the same file.
- A download: choose **Resume…** on the file card or its **Files** row. In the save window that
  opens, choose the same folder and name as before.

### Remove a transfer from the list

**Remove from list** deletes this computer's record of a transfer. The menu explains: "Removes the
record on this computer. Shared files and saved downloads stay."

For a download that did not finish, Crew also cleans up its temporary file:

1. Your system's standard folder window opens. It shows no instruction on macOS. Choose the folder
   you saved counts.csv into.
2. A dialog opens with the bold line "Remove the temporary download for counts.csv?". The line
   under it says that the destination file and the attachment in Crew are kept.
3. Choose **Remove temporary file**, or **Cancel** to keep the record.

### Transfer states

| State | Meaning | What to do |
|---|---|---|
| "Paused" | Stopped by you, or stopped because Biorouter quit during the transfer. | Resume it. |
| "Failed" | Stopped by an error. The reason shows under the row. | Read the reason, then resume it or remove it. |
| "Not confirmed" | The transfer was in its last step when it stopped, so Crew cannot tell whether it finished. It cannot be resumed. | Start the upload or the save again, then use **Remove from list** on the old row. |

If an action on a transfer fails, Crew shows "Crew couldn’t update that transfer." or the reason it
was given.

## Share a path on the lab server

### When to use a server path

Use a server path for a file that is already on the workspace's server, such as a large dataset.
Crew shares only the path. Nothing is uploaded or copied, and there is no size limit on the file.

A server path does not give anyone access to the file. Other people can open it only if their own
account on that server is allowed to read it. Crew does not check that the file exists.

### Share a server path

1. Choose the paperclip button (tooltip "Attach"), then choose **Share a server path…**.
2. A dialog opens titled "Share a file that’s already on labserver", with your server's name as it
   appears in your SSH settings (the settings your computer uses to sign in to servers).
3. In **Path**, type the full path of the file on the server, starting with `/`. The field suggests
   your home folder there, such as `/home/alice/…`.
4. To give the path a friendlier name, open **Advanced** and fill in **Label (optional)**, up to 255
   bytes (fewer characters for accented letters or other scripts). Without a label, Crew uses the
   last part of the path, which is usually the file name.
5. Choose **Add to message**. A chip with a link icon and the label appears in the message box.
6. Press Enter or choose Send.

The path must start with `/`. Otherwise the field reads "Use an absolute path that starts with /."
Crew also refuses a path that contains `..` or is longer than 4,096 bytes.

### What others see

In the message, a server path shows a link icon, its label and a muted "Not uploaded". The tooltip on
"Not uploaded" reads "Crew shares the path only. It doesn’t check that the file exists or grant
access to it." Below it, the full path sits in a box with a **Copy** button that copies the whole
path, even when the box shows only part of it.

A workspace can hold up to 10,000 server paths.

## Messages and files from the command line

This section is for people who use a terminal. Everything here except search can also be done in
the desktop app. For the full command reference, see [Command line](command-line.md).

| Task | Command |
|---|---|
| Post a message | `biorouter crew send '#methods' --text 'Plate reader data is in.'` |
| Post the contents of a text file | `biorouter crew send '#methods' --input notes.md` |
| Read the newest 100 messages | `biorouter crew history '#methods' --latest` |
| Search a channel | `biorouter crew search '#methods' od600` |
| Follow new messages | `biorouter crew watch '#methods'` (Ctrl+C stops following) |
| Mark a channel as read | `biorouter crew channels mark-read '#methods'` |
| Upload a file | `biorouter crew files upload '#methods' counts.csv` |
| Post an uploaded file | `biorouter crew send '#methods' --attachment <attachment ID>` |
| Download a file | `biorouter crew files download --output counts.csv <attachment ID>`. To find the attachment ID, choose **More actions** (⋯) on the file card, then **Copy for support**, then **Copy file ID**. You can also run `biorouter crew history '#methods' --latest --show-ids`. |
| Create a server path reference | `biorouter crew files reference --label 'Raw reads' '#methods' /data/run42/reads.fastq.gz` |
| Post a server path | `biorouter crew send '#methods' --reference <reference ID>` |
| List this computer's transfers | `biorouter crew files pending` |

Points that differ from the desktop app:

- An upload from the command line does not post anything, and the command returns before the upload
  finishes. Run `biorouter crew files pending --show-ids` to see the attachment ID once it is
  finished, then post the file with `send --attachment`.
- Without `--latest`, `history` prints the oldest 100 messages in the channel, not the newest.
- Put quotes around a channel name that starts with `#`. Without them, the shell treats the rest of
  the line as a comment. You can also leave the `#` out: `methods`.
- If you have more than one saved connection, add `--connection` followed by its name.

## When something goes wrong

| What you see | What it means | What to do |
|---|---|---|
| "Couldn’t send." followed by a reason | The workspace did not accept the message. Your text and files are still in the message box. | Fix the cause, then press Send again. The message is not posted twice. |
| "Refresh the workspace to verify connection privacy before sending." | Crew has not finished checking the connection. | Wait for "Connected" in the status row, then send again. |
| "Refresh the workspace to verify connection privacy before uploading." | The same check, for a file. | Wait for "Connected", then add the file again. |
| "The upload couldn’t start." | The upload could not begin. | Try again. If it keeps failing, check the connection in the status row. |
| "The daemon refused this file selection. Choose an accessible file or a new destination filename." | Crew could not accept the file you picked in the file window. | Check that you can open the file and that it is 1 GB or smaller, then try again. |
| "Connection privacy changed. Refresh Crew and choose the file again." | The connection's privacy changed while the file window was open. | Wait for "Connected", then choose the file again. |
| "Message sent, but its upload record couldn’t be cleared. Remove it from Files." | The message was posted. Crew could not remove its local record of the upload. | If the file appears under "Uploaded, not sent" in the **Files** tab, choose **More actions** (⋯) and then **Remove from list**. |
| A red note about a file, with an × | A file you tried to add was refused. | Read the note and follow the matching row in [Files Crew will not share](#files-crew-will-not-share). Choose the × (tooltip "Dismiss") to close the note. |
| A draft is missing after you switched channels | The draft was over 64 KB, you had more than 50 drafts, you quit Biorouter, or the channel's privacy or your access changed. Files are never kept with drafts. | Check the **Files** tab under "Uploaded, not sent" for files you had attached. |
| You cannot find an older file under "In this channel" | Only files in loaded messages are listed. | Scroll up in the message list to load older messages. |
| "That channel isn't available to you. It may be archived, or you may not be in it." | You were removed from the channel, or it was archived. | Ask the channel's owner. |

An upload failure note clears by itself when you edit the message, send, switch channels, or when the
connection's privacy changes.

## Related documentation

- [Crew user manual](README.md): the list of all pages in this manual.
- [Getting started](getting-started.md): what Crew is and how to open it for the first time.
- [Teams, channels and people](teams-channels-and-people.md): creating channels, adding people,
  archiving and the channel menu.
- [Agents and chat access](agents-and-chat-access.md): **Ask my agent**, agent messages and how
  agents read the files you share.
- [Privacy and security](privacy-and-security.md): what "Restricted" and "Public-safe" mean, and
  why Crew refuses credential files.
- [Connections and troubleshooting](connections-and-troubleshooting.md): the status row, the
  connection bar and what to do when Crew cannot connect.
- [Command line](command-line.md): every `biorouter crew` command, including files and transfers.
