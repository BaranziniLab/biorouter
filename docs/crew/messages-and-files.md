# Messages and files

> **What this is.** How to write, read and copy messages in a Crew channel, how Crew keeps unsent
> drafts, and how to share files from your computer or the lab server.
> **Status:** Current.
> **Audience:** Crew members who use the Biorouter desktop app.

This page assumes you have joined a workspace (see [Joining a workspace](joining-a-workspace.md)).
The message box at the bottom of a channel holds the paperclip button (tooltip "Attach"),
**Ask my agent** and the round Send button.

## Write and send a message

1. Choose the message box. It reads "Message #methods".
2. Type your message. Shift+Enter starts a new line. With a Japanese, Chinese or Korean input
   method, Enter confirms the characters and does not send.
3. Press Enter, or choose Send. Send is gray until the message has text, a finished file or a
   server path.
4. The message appears at the bottom, dimmed, with "Sending…", and turns normal once posted.
   Sending also marks the channel as read.

If sending fails, a red note above the box starts with "Couldn’t send." and gives the reason. Your
text and files stay. Fix the cause and press Send again. An unchanged message is never posted twice.

If the reason mentions connection privacy, such as "Refresh the workspace to verify connection
privacy before sending.", wait until the status row reads "Connected", then press Send again. If it
shows another word, such as "Offline", see
[Connections and troubleshooting](connections-and-troubleshooting.md).

A message holds at most 64 KB of text. Attach a long log as a file instead. Nobody can edit
or delete a posted message. There are no threads, reactions or pins. Typing `@bob` does not notify
Bob. Only a terminal can search a channel: see
[Post and read messages](command-line.md#post-and-read-messages).

## Format a message

Crew reads Markdown. The box shows what you type, and the formatting appears once you post.

- `**bold**`, `*italics*`, `~~struck~~`, `` `code` ``, lists, `>` quotes, `---` lines and tables
  work. `- [ ]` makes a checkbox readers cannot tick. `# Title` makes a bold line.
- Three backticks on the lines before and after a block make a code box with **Copy code**.
- Only `http`, `https` and `mailto` addresses become links. They open in your browser.
- An image shows as a link such as "Image: gel". Crew never loads the picture.
- HTML shows as typed.

## Read a channel

- Hover over a message's time to see the full date.
- Long messages fold behind **Show more**.
- A "Restricted" marker means only private models can read that message. See
  [Privacy and security](privacy-and-security.md).
- Agent messages carry an "Agent" badge. When your chat posted one, choose the chat's title to
  open it. **Show details** opens an agent's step updates.
- Crew loads the newest 200 messages. Scroll up to **Older messages** to load more. While you read
  older messages, new ones stay hidden until you choose **Jump to latest**.
- If others post while you are scrolled up, a button such as "3 new messages" goes to the newest
  message.

### Use the keyboard in the message list

Tab enters the list as one stop. Up and Down move between messages. Home and End go to the first
and last loaded message. Tab on a message reaches its buttons and file cards.

## Unread messages

Crew sends no system notifications or sounds. A channel with unread messages shows its name in
bold, with a count, in the Crew sidebar, so open Crew to check.

- A line or day band labeled "New" marks the first unread message. It stays until you leave the
  channel or post there.
- Crew marks a channel read once its newest message has been in view for one second in the active
  window, but never while you read older messages.
- To mark it read yourself, choose the channel name in the header, or right click the open channel
  in the sidebar (Shift+F10), then choose **Mark as read**.

## Copy a message

Point at a message, or reach it with the arrow keys and Tab, and choose the copy button (tooltip
"Copy text"). It shows "Copied". The copy holds the Markdown the author typed. If it shows
"Couldn’t copy", select the text and press Command+C (Ctrl+C on Windows and Linux). **More
actions** (⋯) holds **Copy for support**, with **Copy message ID**, which you need only when support
asks.

## Drafts you leave behind

When you switch channel, team or workspace, or leave Crew, the text in the message box is kept. It
is back when you return.

- A pencil in the sidebar marks a channel that holds a draft.
- Only text is kept. An attached file waits in the **Files** tab under "Uploaded, not sent".
- Drafts are kept in memory only, so quitting Biorouter discards them.
- Crew clears a draft, and says so, when the workspace's or your connection's privacy or
  institution changes, or when you lose access to the channel.

## Share a file from your computer

- The upload starts when you confirm the file. Others see the file only after you press Send.
- A file joins your message once its upload finishes. Text sent before then goes alone.
- Each file can be up to 1 GB. Add several files one after another.
- Crew cannot delete an upload, even one you remove from your message. Check the file first.
- If you upload the wrong file, do not press Send. The copy stays on the server, where the host's
  server account and the server administrators can read it, so tell your host.

### Add a file

1. Choose the paperclip button, then **Upload a file…**. In the file window (the Finder panel on
   macOS), select one file and choose **Open**. You can also drag a file from Finder or File
   Explorer onto the channel, or paste a copied file into the box with Command+V (Ctrl+V on
   Windows and Linux).
2. After a drag or paste, a dialog asks 'Share "counts.csv" (55 KB) to Crew?'. It shows the file's
   full path and a destination such as "#methods in chen-lab". Choose **Share**. Enter or Escape
   chooses **Cancel** and shares nothing.
3. A chip with the file name appears above your text and shows the upload's progress.
4. When "Press Send to share it." appears, press Enter or choose Send. The file appears as a card
   in your message.

While a file uploads:

- A "Paused" or "Failed" chip has a play button. Hover over "Failed" for the reason. Choose play
  and select the same file.
- A note under the chip says when a file with the same name or contents is already in the channel.
  If you send a corrected file under the same name, agents use yours.
- The × on a chip takes the file out of your message. A file you remove, or one that finishes after
  you leave the channel, waits in the **Files** tab.
- After "The upload couldn’t start." or "The daemon refused this file selection…", check that you
  can open the file and that it is 1 GB or smaller, then try again.
- After "Connection privacy changed" or a note that asks you to refresh the workspace, wait until
  the status row reads "Connected", then drop or choose the file again.

### Files Crew will not share

A refused file shows a red note above the box. Its × closes the note.

| Refused | What to do |
|---|---|
| Several files at once | Crew takes the first. Add the others one at a time. |
| A folder | Zip it, or share its files one by one. |
| Over 1 GB | Share a [server path](#share-a-path-on-the-lab-server), or split the file. |
| A picture not saved as a file | Save it as a file first. |
| A shortcut to another folder | Share the original file. |
| A device, socket or pipe | Save the data to an ordinary file. |
| A file that moved, changed or cannot be read | Check that you can open it, then add it again. |
| A credential file, such as `id_rsa` or `.env` | Do not share it. |

The credential check covers every way a file arrives, and can refuse a file after you choose
**Share**. [Privacy and security](privacy-and-security.md) lists what it refuses.

## Open and save a shared file

A shared file appears as a card. When two cards share a name, each shows its post time. Preview
shows a PNG, JPEG, GIF or WebP picture. **More actions** (⋯) holds the download controls and
**Copy for support** (**Copy file ID**, **Copy SHA-256**).

1. Choose Save (tooltip "Save counts.csv") on the card.
2. In the save window (the Finder panel on macOS), choose a folder, then **Save**.
3. If a file with that name exists, the save window asks whether to replace it. Choose **Replace**.
   Crew then asks "Replace counts.csv after the download is verified?". Choose **Replace file**, or
   **Cancel**. The old file stays until the download passes its check.
4. A bar on the card shows "Downloading 42%". When the bar goes away, the file is saved.

Crew checks each download before it saves it. If the check fails, the card gives the reason, and
**Resume…** tries again. Crew never saves into credential or settings folders. "That channel isn't
available to you…" means you are no longer in the channel, or it was archived.

## The Files tab

Choose **Channel details** in the channel header, then **Files**.

| Section | What it lists |
|---|---|
| "In progress" | Unfinished transfers for this channel on this computer, with **Pause**, **Resume…** and **Remove from list** |
| "In your message, not sent yet" | Files in the message you are writing |
| "Uploaded, not sent" | Uploads in no message. **Attach** puts one back into your message. |
| "In this channel" | Files and server paths in loaded messages, with who shared each |

Scroll up in the messages to list older files. If **Attach** says the upload no longer matches,
upload the file again. After "Message sent, but its upload record couldn’t be cleared.", use
**Remove from list** on that file.

## Pause, resume and clear transfers

A transfer is one upload or download on this computer.

- To pause, choose the pause button on the chip, or **Pause** in the card's **More actions** or on
  the "In progress" row.
- To resume, choose the play button on the chip, or **Resume…** on the card or row. Select the same
  file, or for a download the same folder and name.
- **Remove from list** deletes only this computer's record. For an unfinished download, choose the
  folder you saved into, then **Remove temporary file**. The row leaves the list.

### Transfer states

| State | Meaning | What to do |
|---|---|---|
| "Paused" | You paused it, or the background service stopped (a computer restart, or quitting Biorouter on Windows). | Resume it. |
| "Failed" | An error stopped it. The reason shows under the row. | Resume or remove it. |
| "Not confirmed" | It stopped at the end, so Crew cannot tell whether it finished. | Upload or save again, then remove the old row. |

## Share a path on the lab server

For a file already on the workspace's server, such as a large dataset, share its path. Nothing is
uploaded. Readers can open the file only if their own server account may read it. Crew does not
check that it exists.

1. Choose the paperclip button, then **Share a server path…**.
2. In **Path**, type the full path, starting with `/`.
3. To name it, open **Advanced** and fill in **Label (optional)**. By default Crew uses the file
   name.
4. Choose **Add to message**, then press Send. Readers see the label, "Not uploaded" and the path
   with a **Copy** button. A reader copies the path to use the file on the server.

## Related documentation

- [Crew user manual](README.md): every page in this manual.
- [Getting started](getting-started.md): the parts of the Crew view.
- [Teams, channels and people](teams-channels-and-people.md): channels, members and archiving.
- [Agents and chat access](agents-and-chat-access.md): **Ask my agent** and shared files.
- [Privacy and security](privacy-and-security.md): "Restricted" and refused files.
- [Connections and troubleshooting](connections-and-troubleshooting.md): the status row.
- [Command line](command-line.md): every `biorouter crew` command.
- [Administration](administration.md): workspace limits.
