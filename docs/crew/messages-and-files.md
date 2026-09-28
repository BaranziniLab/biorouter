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
The note stays with its draft: if you move to another channel, it comes back with the draft. If the
workspace refuses a message after you moved on, the connection bar says "Couldn’t send your message
in #methods." with the reason.

| Reason | What to do |
|---|---|
| "Messages can be up to 64 KB. Attach long text as a file." | Shorten the message, or attach the text as a file. |
| "Refresh the workspace to verify connection privacy before sending." | Wait until the status row reads "Connected", then press Send again. If it shows another word, such as "Offline", see [Connections and troubleshooting](connections-and-troubleshooting.md). |
| "Nothing was sent. Check the connection, then send again." | Crew lost the connection before the message left. Send again once the status row reads "Connected". |
| "The workspace server can’t save messages right now." | The server's disk is full or failing. Ask the host, who sees what to do in [Administration](administration.md#server-storage-full-or-failing). |
| "This workspace is full…", "This workspace has grown past the size Crew supports…" or "You have used your share of this workspace's…" | The workspace reached a limit. Reading still works. Ask the host ([Workspace limits](administration.md#workspace-limits)). |
| "…was shared in #raw-data. Share it there, or upload it again here." | A file or server path from another channel cannot be posted here. Remove it, then upload it again or share the path again. |
| "This channel is archived, so nothing more can be posted in it." | Copy your text and post it elsewhere. |

If the connection breaks just after you press Send, Crew cannot tell at once whether the message
arrived. The note reads "Checking whether your message reached the channel…" while Crew reads the
channel again. Then it reads "Your message was sent." and the draft goes, or "Couldn’t confirm this
was sent. Check the channel, then send again." and the draft stays. A resent draft is never posted
twice.

A message holds at most 64 KB of text. Attach a long log as a file instead. Nobody can edit
or delete a posted message. There are no threads, reactions or pins. Typing `@bob` mentions Bob:
Bob sees the message marked for him and may get a notification (see
[Unread messages](#unread-messages)). Only a terminal can search a channel: see
[Post and read messages](command-line.md#post-and-read-messages).

## Format a message

Crew reads Markdown. The box shows what you type, and the formatting appears once you post.

- `**bold**`, `*italics*`, `~~struck~~`, `` `code` ``, lists, `>` quotes, `---` lines and tables
  work. `- [ ]` makes a checkbox readers cannot tick. `# Title` makes a bold line.
- Three backticks on the lines before and after a block make a code box with **Copy code**.
- Only public `http` and `https` addresses become links. They open in your browser after you
  confirm. An email address, or an address on a private network such as `localhost`,
  `10.0.0.5` or `wiki.internal`, shows as text with the address beside it, for you to copy.
- When a link's words look like an address on another site, the site it really opens follows
  them in parentheses, such as "lab.example.edu (example.net)".
- An image shows as a link such as "Image: gel". Crew never loads the picture.
- HTML shows as typed.
- A character that draws nothing or reorders the text after it, such as a zero width space or a
  direction override, shows as its code, such as `\u{202e}`. Point at it to read "Hidden
  character U+202E". **Copy text** still copies what the author typed.
- Each paragraph, list item and table cell follows the direction of its own first letters, so
  Arabic, Hebrew or Persian reads right to left.
- A mention of you, such as `@bob` when you are Bob, shows as a highlighted chip, and the message
  gets a colored mark. Your own messages never mark you, and a mention inside code or in a link's
  words does not count.

## Read a channel

- Hover over a message's time to see the full date.
- Long messages fold behind **Show more**.
- A "Restricted" marker means only private models can read that message. See
  [Privacy and security](privacy-and-security.md).
- Agent messages carry an "Agent" badge. When your chat posted one, choose the chat's title to
  open it. **Show details** opens an agent's step updates.
- Crew loads the newest 200 messages. Scroll up to the top to load 200 older ones, which appear
  above the ones you were reading without moving your place. Crew keeps up to 600 messages loaded.
- Past 600, the newest messages give way. A bar then reads "Viewing earlier messages" with
  **Jump to latest**, and **Newer messages** at the bottom of the list loads the next 200. New
  messages stay hidden until you reach the newest again.
- If others post while you are scrolled up, a button such as "3 new messages" goes to the newest
  message.

### Use the keyboard in the message list

Tab enters the list as one stop. Up and Down move between messages. Home and End go to the first
and last loaded message. Tab on a message reaches its buttons and file cards.

## Unread messages

A channel with unread messages shows its name in bold, with a count, in the Crew sidebar. The
**Crew** item in the app sidebar shows how many unread messages all your connected workspaces hold,
and on a Mac the Dock icon shows the same number. Crew checks every 10 seconds.

While you are not looking at Crew, new messages also raise a system notification, such as "3 new
messages in chen-lab", or "Alice Chen mentioned you in #general" when one of them mentions you.
Choosing it opens Crew on that channel. A notification never shows the message's text. Crew shows
at most one a minute for each channel, and six a minute in all. Your computer's notification
settings for Biorouter decide whether they appear and make a sound.

- A channel with unread messages opens at the first of them, where a line or day band labeled
  "New" marks it. The line stays until you leave the channel or post there.
- When more is unread than Crew loaded, the channel opens at the newest message and marks nothing
  read. **Jump to first unread** loads older messages until the first unread one is in view.
- Crew marks messages read once they have been in view for one second in the active window, up to
  the newest message you have seen, never while you read older messages.
- To mark it read yourself, choose the channel name in the header, or right click the open channel
  in the sidebar (Shift+F10), then choose **Mark as read**.
- Reading a channel with `biorouter crew history` or `watch` leaves it unread. See
  [Post and read messages](command-line.md#post-and-read-messages).

## Copy a message

Point at a message, or reach it with the arrow keys and Tab, and choose the copy button (tooltip
"Copy text"). It shows "Copied". The copy holds the Markdown the author typed. If it shows
"Couldn’t copy", select the text and press Command+C (Ctrl+C on Windows and Linux). **More
actions** (⋯) holds **Copy for support**, with **Copy message ID**, which you need only when support
asks.

## Drafts you leave behind

When you switch channel, team or workspace, or leave Crew, the text in the message box is kept. It
is back when you return.

- A pencil in the sidebar marks a channel that holds a draft. While the channel also has unread
  messages, the unread count shows in the pencil's place, and the pencil comes back once you read
  them. A screen reader hears ", draft" after the channel's name either way.
- Only text is kept. An attached file waits in the **Files** tab under "Uploaded, not sent".
- Drafts are kept in memory only, so quitting Biorouter discards them.
- Crew clears a draft, and says so, when the workspace's or your connection's privacy or
  institution changes.
- When you lose access to a channel that held your draft, a note above the message box says
  "#methods held your unsent draft, which can’t be sent there now. Copy it before you close this
  note." Choose **Copy draft**, then **Dismiss**. The draft is gone once you dismiss the note.

## Share a file from your computer

- The upload starts when you confirm the file. Others see the file only after you press Send.
- A file joins your message once its upload finishes. Text sent before then goes alone.
- Each file can be up to 1 GB. Add several files one after another.
- Crew cannot delete a finished upload, even one you remove from your message. Check the file
  first.
- An upload you cancel before it finishes leaves the part already sent on the server for up to a
  day. Until then it counts toward the workspace's file space at the file's full size.
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

- A "Paused" chip has a play button. Hover over it for the reason, such as "You paused it" or
  "The connection dropped". Choose play and select the same file.
- A "Failed" chip means the workspace refused the upload, and resuming cannot fix it. Hover over
  it for the reason.
- A note under the chip says when a file with the same name or contents is already in the channel.
  If you send a corrected file under the same name, agents use yours.
- The × on an uploading chip is **Cancel upload**. Crew asks first, because the part already sent
  stays on the server for up to a day. At "Cancel uploading counts.csv?", choose **Cancel upload**,
  or **Keep uploading** (**Keep it** for a paused upload).
- The × on a finished file's chip takes the file out of your message. A file you remove, or one
  that finishes after you leave the channel, waits in the **Files** tab.
- After "The upload couldn’t start." or "The daemon refused this file selection…", check that you
  can open the file and that it is 1 GB or smaller, then try again.
- After "Your connection is now Private; this file was checked for Public. Refresh Crew and drop
  the file again.", the workspace's privacy changed after Crew checked the file. "Your
  connection's privacy changed since Crew checked it…" means the same. Choose the channel name at
  the top, then **Refresh channel**, and wait until the status row reads "Connected". Then drop
  or choose the file again, if it may still be shared under the new privacy. Do the same after a
  note that asks you to refresh the workspace.

### Files Crew will not share

A refused file shows a red note above the box. Its × closes the note.

| Refused | What to do |
|---|---|
| Several files at once | Crew takes the first, and the note names it: "Crew shares one file at a time. Only counts.csv was added." Add the others one at a time. |
| A folder | Zip it, or share its files one by one. |
| Over 1 GB | Share a [server path](#share-a-path-on-the-lab-server), or split the file. |
| A picture not saved as a file | Save it as a file first. |
| A shortcut to another folder | Share the original file. |
| A device, socket or pipe | Save the data to an ordinary file. |
| A file that moved, changed or cannot be read | Check that you can open it, then add it again. |
| A credential file, such as `id_rsa` or `.env` | Do not share it. |
| A name with an invisible or formatting character, or with characters made to look blank | Rename the file, then share it again. Crew says so, such as "“report�.pdf” has an invisible or formatting character in its name. Rename the file, then share it again.", with each hidden character shown as �. Such a name could hide the file's real type from the people you share it with. |

The credential check runs however a file arrives (the paperclip, a drag, a paste or a terminal),
and can refuse a file after you choose **Share**. It checks the file's name, and reads the file's
first and last 64 KB, so a key added to the end of a large file is caught. A credential in the
middle of a file larger than 128 KB is beyond it, so check large files yourself.
[Privacy and security](privacy-and-security.md) lists what it refuses.

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
**Resume…** tries again. The save window suggests a name Crew accepts. A saved file is marked as
downloaded from the internet, so macOS or Windows checks an app inside it before it opens, as for a
browser download.

If Crew refuses where you chose to save, it says what to change:

| Message | What to do |
|---|---|
| "…starts with a dot, which Crew doesn't save into your home." | Choose a name without the leading dot. |
| "Choose a folder owned by your account that other accounts can't change." | Save into a folder of your own, such as Downloads. |
| "Crew won't save into a credential or settings location. Choose another folder." | Choose another folder. |
| "That is a folder. Give a file name…" | Choose a file name, not a folder. |
| "…is a program, and Crew won't replace one. Choose another name." | Choose another name. |
| "A file named … already exists. Replace it, or choose another name." | Choose **Replace** in the save window, or another name. |

"You're not in that channel." means you were removed from the channel.

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
  folder you saved into, then **Remove temporary file**. The row leaves the list. For an unfinished
  upload, the part already sent stays on the server for up to a day.
- A transfer you pause, resume or remove with `biorouter crew` shows its new state here within a
  few seconds, or when you bring the window back to the front.

### Transfer states

| State | Meaning | What to do |
|---|---|---|
| "Paused" | It stopped and can resume. The reason shows under the row: "You paused it", "The connection dropped", "The credential vault is locked", "Two other transfers were running", "The connection’s privacy changed", "The workspace server couldn’t save it" or, for any other interruption, "It stopped". After a computer restart, or quitting Biorouter on Windows, it shows no reason. | Resume it. After "The connection’s privacy changed", check the connection first, and share or save the file again if Crew refuses. After "The workspace server couldn’t save it", the server's disk is full or failing: choose **Resume…** once the host has freed space and restarted Crew. If you host the workspace, follow [Server storage full or failing](administration.md#server-storage-full-or-failing) first. |
| "Failed" | The workspace refused it. The reason shows under the row. Resuming cannot fix it. | Remove it. Upload or save again once the cause is fixed. |
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
