# Connections and troubleshooting

> **What this is.** How Crew connects your computer to your lab's workspace, what each connection status means, how Crew reconnects by itself, how to sign in, how to change connection settings, and what to do about each error message.
> **Status:** Current.
> **Audience:** Lab members who use Crew in the Biorouter desktop app. No SSH or terminal experience is needed.

Your lab's workspace lives on a shared lab server. Your computer reaches that server over SSH, the secure login that lab servers use. The Biorouter background service on your computer opens that connection, keeps it open, and checks that the server and the workspace are the ones you joined. When the connection is up, you see your channels. When it is down, Crew tells you why and offers the action that fixes it.

In this manual, a word in braces stands for a name that Crew fills in. For example, "Connect to {workspace}" appears on your screen as "Connect to chen-lab" when your workspace is called `chen-lab`. On this page, {server} is your lab server's name (such as `lab-server`), and {server address} is the server's address (such as `192.0.2.10`). The next section explains the difference.

## How Crew names your server

Crew shows your server under one of two names. Both names mean the same server.

| Name | Example | Where Crew uses it |
|---|---|---|
| The name from your own SSH settings | `lab-server` | The workspace menu, the "Connecting to" screen, the offline screen, the "Sign in to" screen, the You row, most dialogs |
| The server's address | `192.0.2.10` | The title of the Sign in window, the connection bar, the server identity screens, the "Crew isn’t set up" screen |

Crew uses the first name only when your SSH settings file (`~/.ssh/config`) has an entry for that server. Without one, Crew shows the address everywhere. In **Connection settings…**, the **Your server login** field keeps the address, and a line under it reads "Your SSH settings call this server {name}."

## Where connection problems appear

Crew reports the state of the connection in four places inside Crew, and in one place outside it.

| Place | Where it is | What it shows |
|---|---|---|
| Status row | At the top of the Crew sidebar, under the workspace name | One status word, with a dot or a spinning circle. The privacy chip sits at its right. |
| Workspace menu | Choose the workspace name at the top of the Crew sidebar | Who you are signed in as, the status, the last error in words, and the connection tools. |
| Main area | The large area in the middle of the Crew view | One screen for the problem, with one button to fix it. |
| Connection bar | A strip at the top of the channel column, under the channel header | Error notes with **Retry**, **Try again** or a close button, and notices such as a locked vault. |
| Chats outside Crew | A bar above an ordinary Biorouter chat that has Crew access | That Crew is offline, with **Connect in Crew** or **Connect now**. |

## Status row states

The status row shows exactly one word at a time. If more than one state applies, Crew shows the most important one. A server that Crew cannot verify outranks every other state.

| Status | What it means | What to do |
|---|---|---|
| "Connected" | The connection is up, and Crew verified the workspace's identity. Screen readers hear "Connected · identity verified". | Nothing. |
| "Connecting…" | Crew is connecting now, or the Sign in window is open. A spinning circle replaces the dot. | Wait. If the Sign in window is open, sign in. |
| "Checking connection" | The connection is up, and Crew is loading your channels for the first time. The main area shows placeholder shapes. | Wait. |
| "Updating…" | Live updates for a workspace you already saw stopped, and Crew is checking the workspace again. The privacy chip shows "Checking privacy…". | Wait. |
| "Reconnecting…" | The live view dropped, and Crew is finding out whether the connection is still up. A spinning circle replaces the dot. This word appears only when that takes longer than 0.3 seconds. | Wait. |
| "Updates unavailable" | The connection is up, but Crew is not receiving live updates. Hovering over the word shows "Crew isn’t receiving updates for {workspace}. Retry below." | Choose **Retry** in the connection bar. |
| **Sign-in needed** | The server wants your password or a verification code. This word is a button. | Choose **Sign-in needed**, then sign in. See [Sign in](#sign-in). |
| "Can’t verify server" | Crew could not confirm that the server, or the workspace on it, is the one you trust. | Read the screen in the main area. See [Server identity checks](#server-identity-checks). |
| "Not set up on this server" | Your computer reached the server, but Crew is not installed for your account there, or Crew did not start after you signed in. | See [Crew is not set up on the server](#crew-is-not-set-up-on-the-server). |
| "Not joined yet" | The connection is up, and you are waiting for your host to let you in. Hovering over the word shows "Waiting for {host} to let you in". | Send your code to your host and wait. See [Joining a workspace](joining-a-workspace.md). |
| "Offline" | The workspace is not connected. You see this in four cases. The background service started again, for example after your computer restarts (see [When you reopen Biorouter](#when-you-reopen-biorouter)). You chose **Disconnect**. You saved a change in **Connection settings…**. The background service noticed that the connection dropped, for example because your computer went to sleep or the network went down. In that last case, Crew reconnects by itself for up to an hour (see [Automatic reconnection](#automatic-reconnection)). | Choose **Connect to {workspace}** in the main area. After a dropped connection, you can also wait for Crew to reconnect. Choosing **Connect to {workspace}** tries at once. |
| "Can’t connect" | The last attempt to connect failed. A network failure also shows this word. The connection bar then reads "Can’t reach {server address}." | Read the connection bar and the main area. See [Troubleshooting](#troubleshooting). |

Three status words show a tooltip when you hover over them with the pointer: "Connected" shows "Connected · identity verified", and "Updates unavailable" and "Not joined yet" show the text in the table above. Screen readers hear the same text as part of the status.

The privacy chip at the right of the status row shows the workspace's privacy only when Crew has confirmed it. While the status is a problem state, the chip is empty. See [Privacy and security](privacy-and-security.md).

## Screens in the main area

When there is a connection problem, the main area shows one screen for it. Most screens have one button that fixes the problem. A screen has no button when there is nothing for you to do.

| Screen title | When it appears | Button |
|---|---|---|
| "Connecting to {server}…" | A connection attempt is running, or the Sign in window is open | None |
| "{workspace} is offline" | The status is Offline or Can’t connect | **Connect to {workspace}** |
| "Sign in to {server}" | The server asked for a password or a code, and you closed the Sign in window before finishing | **Sign in** |
| "Can’t verify {server address} yet" | The server's identity is not in your computer's list of trusted servers | **Try again** |
| "{server address}’s identity changed" | The server's identity is different from the one your computer trusts | **Copy details for IT** |
| "This isn’t the workspace you joined" | The server answered with a different workspace key | **Copy details**, **Connection settings…** |
| "Crew isn’t set up for your account on {server address}" | Crew is not installed for your account on the server | **Try again** |
| "Messages will show here again once live updates are back." | Live updates stopped before your channels loaded | None. Use **Retry** in the connection bar. |

### The offline screen after a failed attempt

The offline screen reads "{workspace} is offline" and "Connect to see your channels." When you choose **Connect to {workspace}** and the attempt fails, a line appears under the button:

"Tried again at {time}. {reason}."

The time includes seconds, for example 3:04:17 PM, so you can see that a repeated try did run. The reason is one of these:

| Reason | Cause |
|---|---|
| "Couldn’t reach {server}" | A network problem. Your computer could not reach the server. |
| "{server} asked you to sign in" | The server wants your password or a verification code. |
| "Crew couldn’t verify {server}" | The server's identity could not be confirmed. |
| "Crew isn’t running for you on {server}" | Crew is not installed for your account on the server. |
| "It didn’t connect" | Any other failure. One common cause is that Crew stopped on the server. See the paragraph under this table. |

After a network failure, the line also says "Crew keeps trying by itself while the network is down." Crew never says this when the server asked you to sign in, when it could not verify the server, or when Crew is not set up, because nothing retries those by itself.

If you connected before and now see "It didn’t connect" each time, Crew may have stopped on the server, for example after the server restarted. Crew does not start again by itself on the server. Ask your host to start it again (see [After the server restarts](hosting-a-workspace.md#after-the-server-restarts) in Hosting a workspace). When your host tells you it is running, choose **Connect to {workspace}**.

## Connect and disconnect

### Connect to a workspace

1. Open Crew. If you have more than one workspace, choose the workspace name at the top of the Crew sidebar, then choose the workspace under **Switch workspace**.
2. In the main area, choose **Connect to {workspace}**. For example, **Connect to chen-lab**.
3. The main area shows "Connecting to {server}…", and the status row shows "Connecting…".
4. If the server asks for a password or a verification code, the Sign in window opens by itself. Follow [Sign in step by step](#sign-in-step-by-step).
5. When the connection is up, the status row shows "Connected", and your channels appear.

If the attempt fails, the main area and the connection bar say why. Find the message in [Troubleshooting](#troubleshooting).

Several buttons start the same kind of connection attempt as **Connect to {workspace}**. If the server asks for a password or a code during that attempt, the Sign in window opens by itself.

| Button | Where |
|---|---|
| **Connect to {workspace}** | The offline screen |
| **Reconnect** | The workspace menu, and the join screen while you wait for your host |
| **Try again** | The connection bar, the "Can’t verify" screen, and the "Crew isn’t set up" screen |
| **Connect in Crew**, **Connect now** | The bar above an ordinary chat that has Crew access |

### When you reopen Biorouter

What you see depends on whether the background service kept running while Biorouter was closed.

On a Mac or a Linux computer, quitting Biorouter leaves the background service running. The service keeps your workspace connected, and it keeps reconnecting after a network problem. When you open Biorouter again:

1. A window titled "Connect to existing BioRouter daemon" asks for your approval secret. Type it. See [The approval secret](getting-started.md#the-approval-secret).
2. Open Crew. The workspace shows its current status. If the connection stayed up, the status row shows "Connected", and your channels appear without a click.
3. If you are still waiting for your host, the card with your code comes back with the same code. If your host let you in while Biorouter was closed, Crew finishes joining by itself.
4. If the status is anything else, follow the screen in the main area.

When the background service has stopped, each workspace shows "Offline" when you open Biorouter again. This happens after your computer restarts. On Windows, it also happens every time you quit Biorouter, because quitting stops the background service there.

1. On a Mac or a Linux computer, Biorouter asks you to set an approval secret in two windows: "Set approval secret for shared BioRouter daemon" and "Confirm shared daemon approval secret". Type the same secret in both. You can use the secret you used before.
2. Open Crew, and choose **Connect to {workspace}**.
3. If the server asks for a password or a code, sign in. See [Sign in step by step](#sign-in-step-by-step).

### Disconnect from a workspace

1. Choose the workspace name at the top of the Crew sidebar.
2. Choose **Disconnect**. Crew does not ask you to confirm.

The status changes to Offline, and the main area shows "{workspace} is offline". Crew does not reconnect by itself after you disconnect. **Disconnect** is unavailable while a connection attempt runs.

If you are still waiting for your host to let you in, disconnecting does not change your code. The menu says so under the item: "Stop waiting for now. {host} can still let you in with the same code."

### The workspace menu

Choose the workspace name at the top of the Crew sidebar to open the workspace menu. The top of the menu describes the connection:

1. The workspace name.
2. "Hosted by" and the host's name, when Crew knows it.
3. When connected: "Signed in as @{username} on {server}". Before then: "Server" and the server's name.
4. The status, or "Connected · identity verified" when connected.
5. When connected: "Fingerprint", the first part of the workspace fingerprint, and a small **Copy** button that copies the whole fingerprint.
6. When not connected: the last error, in words.

The connection tools are below the workspace items:

| Item | When the menu lists it | What it does |
|---|---|---|
| **Reconnect** | Whenever the status is not Connected. It is unavailable while a connection attempt runs, while the Sign in window is open, or while a disconnect runs. | Starts a connection attempt. |
| **Sign in…** | Only when the status is Sign-in needed | Opens the Sign in window. |
| **Disconnect** | Always. It is unavailable while a connection attempt or a disconnect runs. | Closes the connection. |
| **Connection settings…** | Always | Opens [Connection settings](#connection-settings). |

**Reconnect** tries the connection again. **Sign in…** opens the window where you type your password or code. They are different actions.

While you wait for your host to let you in, a note sits under **Reconnect**: "Try the connection again. Your code doesn’t change."

When some workspace items are unavailable, a note above them says why:

| Note | Meaning |
|---|---|
| "Available after you join" | Your host has not let you in yet. |
| "Available once the connection is verified" | The connection is up, and Crew is still verifying the workspace. |
| "Available once you’re connected" | The workspace is not connected. |

With two or more saved workspaces, the menu lists them under **Switch workspace**. Each workspace in that list has a colored status dot. Screen readers also hear its status in words. For the workspace you have open, that is its current word from the status row (see [Status row states](#status-row-states)). For every other workspace, it is Connected, Sign-in needed, Can’t connect or Offline. The list is unavailable while Crew is busy connecting.

## Automatic reconnection

Crew keeps an idle connection open. After a network problem, it reconnects by itself for up to an hour. You do not need to press anything when the network comes back.

### Keeping an idle connection open

The lab server closes a Crew connection that is silent for 5 minutes. To prevent that, the background service sends a small check over the connection after 2 minutes without activity. The check confirms that the connection still reaches the same workspace. It reads nothing from your channels and writes nothing to the workspace.

When the connection's SSH process ends, for example because your computer went to sleep or the network dropped, the background service notices within 5 seconds.

### Reconnecting after a network problem

When a connection drops because of the network, the background service dials the server again on this schedule:

| Try | Wait before the try |
|---|---|
| First | 20 seconds |
| Second | 60 seconds |
| Third | 3 minutes |
| Later tries | Every 5 minutes, for up to one hour |

Each try connects the same way **Connect to {workspace}** does, and checks the server and the workspace again. It never asks for a password or a code.

The schedule starts whichever way the drop is found: by the background check, by Crew reading the workspace, or by your own **Connect to {workspace}** failing because of the network.

### What you see while Crew reconnects

1. The status row may show "Reconnecting…" while Crew works out what happened.
2. If the connection is down, the status row shows "Offline" or "Can’t connect", and the connection bar may show "Can’t reach {server address}."
3. If a connection attempt takes longer than one second, the connection bar shows "Reconnecting to {workspace}…".
4. While Crew shows the workspace as offline, the app reads the connection's state every 15 seconds while its window is visible. It also reads it at once when you come back to the window and when your computer's network comes back. It keeps reading for up to 60 minutes. Reading does not connect anything. The app does not do this after you choose **Disconnect**.
5. When the background service has reconnected, your channels come back without a click.

### When Crew does not reconnect by itself

Crew stops trying, or never starts, in these cases:

| Case | What to do |
|---|---|
| You chose **Disconnect**. | Choose **Connect to {workspace}** when you want to connect. |
| You saved a change in **Connection settings…**, or removed the workspace. | Choose **Connect to {workspace}**. |
| The server wants a password or a verification code. Automatic tries cannot type one. | Choose **Connect to {workspace}**. The Sign in window opens. |
| The Sign in window is open and waiting for you. | Finish signing in, or choose **Close**. |
| Crew could not verify the server or the workspace. | See [Server identity checks](#server-identity-checks). |
| Crew is not installed for your account on the server. | See [Crew is not set up on the server](#crew-is-not-set-up-on-the-server). |
| The workspace removed your computer or your account. | Ask your host. |
| The network stayed down for more than an hour. | Choose **Connect to {workspace}**. |
| The background service stopped, for example because your computer restarted, or because you quit Biorouter on Windows. | Open Biorouter and choose **Connect to {workspace}**. See [When you reopen Biorouter](#when-you-reopen-biorouter). |

If your server asks for a password or a verification code each time, you sign in again after every dropped connection.

### Live updates that stop while the connection stays up

Sometimes the live view of your channels ends while the connection itself stays up. Crew then picks the view up again quietly: at once, then after 20 seconds, then after 60 seconds. It does this at most three times in 10 minutes. If the view keeps dropping, the connection bar shows the problem with a **Retry** button. See [Live updates](#live-updates).

### A request that was in progress when the connection broke

If the connection breaks while it carries a request, such as a message you sent, Crew never sends that request again by itself. The workspace menu may then show this last error: "SSH bridge failed. Reconnect; inspect any submitted operation before retrying because its outcome may be unknown." After you reconnect, check whether your message or action went through before you repeat it.

### Chats outside Crew

An ordinary Biorouter chat that has access to a Crew channel shows a bar when Crew is offline:

| Bar text | Button |
|---|---|
| "Crew is offline. It will reconnect by itself when the network is back." | **Connect now** |
| "Crew is offline. This chat can’t read or post in {channel} until you connect." | **Connect in Crew** |

The first text appears when the network caused the problem. You do not need to press anything, because Crew reconnects by itself. Both buttons open Crew and connect the workspace there.

When Crew opens, it connects only a workspace that is disconnected for an ordinary reason. It does not connect when the server needs you to sign in, when Crew could not verify the server, or when your membership ended. Crew then shows the screen for that problem instead. The bar also offers **Revoke access**. For more about chat access, see [Agents and chat access](agents-and-chat-access.md).

## Sign in

### When Crew asks you to sign in

Some lab servers ask for a password, a verification code, or both, before they let you connect. A verification code is the short code your institution's login system gives you each time you sign in. Crew shows the server's own questions in a small terminal window, and you type your answers there.

The Sign in window opens by itself when you start a connection attempt and the server asks for a password or a code. Automatic reconnection never opens it.

You can also open the Sign in window in three other ways:

- Choose **Sign-in needed** in the status row.
- Choose **Sign in…** in the workspace menu.
- Choose **Sign in** on the "Sign in to {server}" screen in the main area.

### Sign in step by step

1. The Sign in window opens. Its title is "Sign in to {server address}", and under it: "Type your password or verification code in the box below. Nothing you type is saved."
2. Wait for the server's question to appear in the box, for example "Password:". Crew puts the keyboard cursor in the box.
3. When the server asks for your password, type it and press Return. The server does not show the characters of a password as you type them.
4. If the server asks for a verification code, type the code and press Return.
5. If the server shows a numbered list of options, for example "Passcode or option (1-3):", type the number of the option you want and press Return. If that option sends a request to your phone, approve the request on your phone.
6. When the server accepts you, the window closes by itself. Crew connects, and your channels appear. You do not need to choose Connect again.

The server writes these questions, not Crew, so their wording depends on your institution. Crew's testing used servers that accept SSH keys only. Signing in with a phone approval has not been tested.

While the window is open, the status row shows "Connecting…", and the main area shows "Connecting to {server}…".

The window has no close button in its corner. Pressing Escape or clicking outside the window does nothing. To leave, choose **Close** at the bottom of the window. **Close** stops signing in. The main area then shows "Sign in to {server}" and "The server needs your password or a verification code." with a **Sign in** button, so you can start again.

### Help inside the Sign in window

Choose **Trouble signing in?** under the box. It says:

- "Use the same username and password you use for this server."
- "If your IT team gave you a jump host, add it in Connection settings." A jump host is a gateway server that your institution requires you to pass through before you reach the lab server. Enter it in the **Jump hosts** field of [Connection settings](#connection-settings).
- "Crew checks servers against this file:" followed by the path of your computer's list of trusted servers, `~/.ssh/known_hosts`, with a copy button.

### When signing in does not finish

If the server refuses your answers, or signing in ends early, an error note appears under the box. For example: "SSH authentication ended (exit {code}). Choose Reconnect to check the connection." To follow it:

1. Choose **Close** in the Sign in window.
2. Choose the workspace name at the top of the Crew sidebar.
3. Choose **Reconnect**.

If the box shows "Authentication refused. Close and reconnect.", the background service refused to finish signing in. One cause is that you signed in to the server, but Crew could not start there. Choose **Close**, then **Reconnect**. If the main area then shows "Crew isn’t set up for your account on {server address}", see [Crew is not set up on the server](#crew-is-not-set-up-on-the-server).

Every message this window can show is in [Messages in the Sign in window](#messages-in-the-sign-in-window).

## Server identity checks

Every server has a key that proves its identity, called a host key. Your computer keeps a list of the servers it trusts in the file `~/.ssh/known_hosts`. Crew connects only to a server whose host key is already on that list. Crew also checks that the workspace on the server has the same workspace key as the invitation you joined with.

When a check fails, the status row shows "Can’t verify server", and the main area shows one of three screens. None of them has a button that accepts a new key. Crew never adds a server to your trusted list for you.

### Crew cannot verify the server yet

The screen reads "Can’t verify {server address} yet" and "Crew only connects to servers you’ve already verified." This means the server's host key is not in your list of trusted servers. You see it when your computer has never been set up to trust this server.

When SSH reported it, the screen shows "Fingerprint the server offered" with a copy button. Choose **How do I verify it?** to see these steps:

1. "Get {server}’s fingerprint from your IT team or your institution’s directory. Check jump hosts too."
2. "Compare it using your usual SSH setup, then add the full key to your known-hosts file. A fingerprint alone isn’t enough."
3. "Come back and choose Try again."

Under the steps, **Open a terminal here** opens a terminal on your computer inside Crew, for whoever adds the key. **Hide terminal** closes it again.

To fix it:

1. Ask your IT team for the server's fingerprint. It starts with `SHA256:`.
2. Follow [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer) in Joining a workspace. It lists the commands to type and what SSH shows. You do it once for each server, on each computer, so you need it again on a new computer.
3. Choose **Try again**.

If you have not used a terminal before, ask your IT team to do steps 1 and 2 with you.

### The server's identity changed

The screen reads "{server address}’s identity changed" and "Don’t connect until your IT team confirms this change. Crew won’t connect while the old key is in your known-hosts file." It may also show "Fingerprint Crew knew" and "Fingerprint the server offered now".

A changed host key can mean the server was rebuilt or replaced. It can also mean that something is pretending to be your server. Treat it as a security problem until IT says otherwise.

1. Choose **Copy details for IT**. The button changes to "Copied". The copied text starts with "Server: {server address}" and "Problem: The server’s host key changed since Crew last connected.", followed by SSH's own message.
2. Paste the details into a message to your IT team.
3. Wait for IT to confirm the change and to update your list of trusted servers.
4. Choose the workspace name at the top of the Crew sidebar, then choose **Reconnect**.

If copying fails, the button reads "Copy failed", and the details appear in a text box so you can select and copy them yourself.

### This is not the workspace you joined

The screen reads "This isn’t the workspace you joined" and "The server answered with a different workspace key. Don’t continue until {host} confirms what changed." Here {host} is the person who hosts the workspace, such as Alice Chen (@alice).

1. Choose **Copy details**. The copied text includes "Problem: The server answered with a different workspace key than the one saved."
2. Send the details to your host and ask what changed.
3. If the connection points to the wrong server, choose **Connection settings…** to check it.

If your host confirms that the workspace was created again, you need a new invitation. See [Joining a workspace](joining-a-workspace.md).

## Crew is not set up on the server

Crew needs a small program named `biorouter-crew` in each person's own account on the lab server. It is installed once for each account, usually by the host or the IT team. When it is missing, the main area shows:

- The title "Crew isn’t set up for your account on {server address}".
- "It’s installed once per account, usually by your host or IT team."
- A ready message to send to your host, with a copy button. It reads like this: "Hi Alice, Crew isn’t set up for my account (@bob) on 192.0.2.10 yet. Could you or IT install biorouter-crew in ~/.local/bin for me?"
- **Install it yourself**, which holds commands for people who use a terminal on the server.
- A **Try again** button.

To fix it:

1. Copy the message and send it to your host or your IT team.
2. When they tell you it is installed, choose **Try again**.

**Install it yourself** says "Get a verified biorouter-crew file for this server, then run:" followed by these commands. Replace the path in the first line with the path of the verified file on the server. Run the commands in a terminal signed in to the server as yourself.

```bash
VERIFIED_BINARY='/replace/with/path/to/verified/linux/biorouter-crew'
mkdir -p "$HOME/.local/bin"
install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" --version
```

The program must be at exactly `~/.local/bin/biorouter-crew` in your account. A copy in `/usr/local/bin` or anywhere else on the server does not count. For more, see [Administration](administration.md).

## Connection settings

Connection settings holds how your computer reaches one workspace: the server login, the privacy setting, and optional SSH details.

### Open Connection settings

Choose the workspace name at the top of the Crew sidebar, then choose **Connection settings…**. The item is always there.

**Connection settings…** also appears in the connection bar next to "Can’t reach {server address}.", on the "This isn’t the workspace you joined" screen, and in the setup checklist.

The window opens with the cursor at the end of the **Connection name** field. To close it without saving, press Escape, choose the close button in its corner, or choose **Cancel**. Clicking outside the window does not close it. While Crew saves, the window cannot be closed.

### Fields

| Field | What to enter | Messages you may see |
|---|---|---|
| **Connection name** | The name this computer shows for the workspace. Required. | None |
| **Your server login** | Your username and the server, in the form `you@server.example.edu`. Crew fills this in when you join. Change it only if your IT team tells you to. | "Your SSH settings call this server {name}." |
| **Privacy** | **Private** ("Only private and institution-approved models") or **Public** ("Public models allowed for public-safe work"). | See [Changing privacy](#changing-privacy). |
| **Institution** | Shown only for Private. Your organization's short ID, such as `ucsf`. Required for Private. | While empty: "Your organization’s short ID, as your host uses it." If the ID has the wrong form: "Use lowercase letters, numbers, hyphens and underscores, starting with a letter or number." |

**Advanced** holds optional SSH details. Its summary line shows what is set, for example "Port 22 · your SSH settings". Advanced opens by itself when the connection already uses one of these fields.

| Field | What to enter | Messages you may see |
|---|---|---|
| **Port** | The SSH port. The default is 22. | "Use a port from 1 to 65535." |
| **Identity file** | The full path of the SSH key file to use. It must start with `/`, such as `/Users/you/.ssh/id_ed25519`. The empty field shows `~/.ssh/id_ed25519` as a hint, but Crew refuses a path that starts with `~`. Leave it empty to use your SSH settings. | "Identity file must be an absolute path" |
| **Jump hosts** | The gateway server your IT team gave you, such as `gateway.example.edu`. | None |
| **Remote work folder** | A folder on the server that your agent may use, written as a full path that starts with `/`, such as `/home/you/project`. | "Use an absolute path that starts with /." |
| **Let my agent run commands in this folder** | A switch. It is available only when a remote work folder is set. | "Set a remote work folder first." |

For more about the agent settings, see [Agents and chat access](agents-and-chat-access.md).

**Workspace details** ("IDs for support") holds the workspace fingerprint and buttons that copy the workspace ID, socket path, device ID and cluster ID. Use them when support staff ask for them. It stays closed until you open it.

### Save changes

1. Change the fields you need.
2. Choose **Save connection**, or press Return in a field.
3. If a hidden field in **Advanced** has a problem, Crew opens **Advanced** and moves the cursor to that field. Fix it and save again.

If nothing changed, saving does nothing, and the connection stays up.

If something changed, Crew disconnects the workspace and then saves. After that:

- The workspace shows Offline. Choose **Connect to {workspace}** to connect with the new settings. Crew does not reconnect by itself.
- Chats outside Crew that had access to channels in this workspace lose that access. So do chats with access to other workspaces that this computer reaches on the same server. The chat says "Crew settings changed since this chat was given access to {channel}, so it can’t continue. Grant access again to continue, or start a new chat." Choose **Grant access again** in the chat, or start a new chat.

When the background service refuses a change, it refuses before it disconnects anything, and the message appears inside the window.

### Changing privacy

When you change a Private connection to Public, Crew asks you to confirm:

1. The window "Make your {workspace} connection public?" opens. It says "Public models will be able to read public-safe work you can see here. Restricted content stays private. Your unsent draft will be cleared."
2. Type the workspace's name in the field "Type {workspace} to confirm".
3. Choose **Make public**. Choose **Cancel** to keep the connection Private. Cancel sends nothing.

Workspaces that this computer reaches on the same server share one privacy setting. If any of them is Private, Crew keeps all of them Private. One computer also cannot use two institutions on the same server. For what Private and Public allow, see [Privacy and security](privacy-and-security.md).

### Remove a workspace from this computer

1. Open **Connection settings…**.
2. Choose **Remove {workspace} from this computer…** at the bottom of the window.
3. A window asks "Remove {workspace} from this computer?" and says "Chats connected to it lose access. Your messages stay on the server, and you can add it again."
4. Choose **Remove**, or **Cancel** to keep the workspace.

Removing disconnects the workspace, ends the access of every chat connected to it, and deletes this computer's device key for the workspace. It does not delete anything on the server.

## Keyboard and screen reader notes

- The status row is a status region named "Connection status". Screen readers announce changes to it.
- The Sign in window puts the keyboard cursor in the terminal box once the server connection opens.
- When **Retry**, **Try again** or the close button in the connection bar removes its own note, the keyboard focus moves to the channel heading. If there is no channel heading, it moves to the message box, then to the first control in the Crew sidebar.
- The copy button next to the fingerprint in the workspace menu is not the first stop. Press the Up arrow from the first menu item to reach it.

## From the command line

The `biorouter crew` command in a terminal has the same connection tools. The help text of each command:

| Command | Help text |
|---|---|
| `biorouter crew status` | Show saved connections and their daemon-reported state |
| `biorouter crew connect` | Open the verified SSH bridge for the selected connection |
| `biorouter crew disconnect` | Close the selected SSH connection |
| `biorouter crew auth` | Authenticate SSH through the shared daemon's owned authentication session |

`biorouter crew auth` signs you in from a terminal. It connects the workspace after you sign in, and prints "Authenticated. The connection is ready." With more than one saved workspace, add `--connection NAME` before the command. For every command, see [Command line](command-line.md).

## Troubleshooting

Find the message you see in the tables below. Each table covers one place on the screen. **Try again** in the connection bar, **Reconnect** in the workspace menu and **Connect to {workspace}** on the offline screen do the same thing.

### Connection bar

| Message | Cause | What to do |
|---|---|---|
| "Can’t reach {server address}." | Your computer could not reach the server. The server name could not be found, the server refused or did not answer, or your computer has no route to it. | Check that your computer is online. If your institution requires a VPN (virtual private network, the app that connects your computer to your institution's network) to reach the server, connect to it. Crew keeps trying for an hour. To try at once, choose **Try again**. If the address or port is wrong, choose **Connection settings…**. |
| "Can’t connect to {server address}." | The connection attempt failed for a reason Crew could not name. | Choose **Try again**. If it worked before and still fails, Crew may have stopped on the server, for example after the server restarted. Ask your host to start it again (see [After the server restarts](hosting-a-workspace.md#after-the-server-restarts) in Hosting a workspace). Otherwise, check **Connection settings…** with your IT team. |
| "Crew can’t connect." | The same, when Crew has no server name to show. | Same as above. |
| "{server address} asked you to sign in." | The server wants your password or a verification code. | Choose **Sign-in needed** in the status row. See [Sign in](#sign-in). |
| "The server asked you to sign in." | The same, when Crew has no server name to show. | Same as above. |
| "Crew couldn’t verify {server address}." | A server identity check failed. | Open Crew's main area and follow its screen. See [Server identity checks](#server-identity-checks). |
| "Crew couldn’t verify the server." | The same, when Crew has no server name to show. | Same as above. |
| "Crew isn’t running for you on {server address}." | Crew is not installed for your account on the server, or it did not start after you signed in. | See [Crew is not set up on the server](#crew-is-not-set-up-on-the-server). |
| "Crew isn’t running for you on the server." | The same, when Crew has no server name to show. | Same as above. |
| "You already use this server for {other workspace} ({institution}). {workspace} uses {other institution}; one computer can't mix institutions on the same server." | Two workspaces on this computer use the same server with different institutions. | Use one institution for each server on this computer. Ask your host which institution is right. |
| A sentence that starts with "Crew SSH host" and says "requires" or "uses a custom ProxyCommand". For example: "Crew SSH host gateway requires StrictHostKeyChecking yes; put this in its matching Host stanza before broader defaults", "Crew SSH host {host} uses a custom ProxyCommand; use native ProxyJump so every SSH hop can be checked", or "Crew SSH host {host} requires GSSAPIDelegateCredentials no; delegated credentials are not admitted". | Your SSH settings file (`~/.ssh/config`) lacks a setting that Crew requires for that server or jump host, or it uses a setting that Crew refuses. Crew checks this before it connects and does not try again by itself. The workspace menu shows the same sentence as its last error, and the offline screen reads "It didn’t connect". | Copy the sentence and send it to your IT team with a link to [SSH](administration.md#ssh) in Crew administration. After they fix your SSH settings, choose **Connect to {workspace}** on the offline screen or **Reconnect** in the workspace menu. |
| "Reconnecting to {workspace}…" | A connection attempt has run for more than one second. | Wait. |
| "Your Crew vault is locked." | This computer keeps Crew's keys in an encrypted vault instead of the system keychain, and the vault is locked. | Choose **Unlock**. A window titled "Unlock Crew encrypted vault" asks: "Enter this Crew vault’s passphrase. This is separate from the daemon approval secret." Enter the passphrase you chose for the vault. |
| A note that includes "Vault unlock was refused. Check the passphrase and selected profile." | The background service refused the passphrase. | Choose **Unlock** and enter the passphrase again. |
| "Crew couldn’t unlock the vault." | The vault stayed locked, and no reason was given. | Choose **Unlock** again. |
| "A new device was added to your account on {date}." | Another computer was added to your account in this workspace. | Choose **Review**. It opens **Keys and security**. If you did not add that device, tell your host. See [Privacy and security](privacy-and-security.md). |
| "A new device was added to your account." | The same, when Crew does not know the date. | Same as above. |

A connection note in the bar goes away by itself as soon as the connection is verified again, whoever reconnected it. While the offline screen shows its own **Connect to {workspace}** button, the bar note has no **Try again** button. The bar never shows the raw technical text of an SSH failure.

### Live updates

These notes appear in the connection bar when the connection is up but the live view of your channels stopped. Most have a **Retry** button. **Retry** first checks the connection. If the connection has dropped, **Retry** connects at once.

| Message | Cause | What to do |
|---|---|---|
| "Live updates for {workspace} stopped." | The live view ended, and picking it up again did not work. | Choose **Retry**. |
| "Live updates stopped." | The same, without a workspace name. | Choose **Retry**. |
| "Live updates keep stopping. Check that Crew is running, then retry." | Three quick attempts to pick up the view failed. | Wait a moment, then choose **Retry**. If it keeps happening, quit and reopen Biorouter. |
| "Crew sent updates for a different workspace, so they weren’t shown." | An update arrived for a workspace other than the one on screen. | Choose **Retry**. |
| "Earlier messages couldn’t be loaded." | Older messages in the channel did not load. | Choose **Retry**. |
| "Crew couldn’t load your saved workspaces." | Crew could not read the list of workspaces on this computer. | Choose **Retry**. If the main area shows **Try again** instead, choose it. |
| "You no longer have access to {channel}." | Someone removed you from the channel, or its access changed. | Ask the channel owner if you need access again. |
| "You no longer have access to {channel}, so it was closed." | The same, after Crew closed the channel. | Choose the close button on the note. Ask the channel owner if you need access again. |
| "You no longer have access to that channel, so it was closed." | The same, without a channel name. | Same as above. |
| "Your access to {workspace} changed." | The workspace changed what you may see, or its privacy setting changed. | Choose **Retry**. If it continues, ask your host. |
| "{workspace} doesn’t recognize this computer yet." | The workspace does not know this computer. This is normal while you wait for your host to let you in. | See [Joining a workspace](joining-a-workspace.md). |
| "{workspace} doesn’t recognize this computer any more. If you didn’t expect that, ask the host." | The workspace knew this computer earlier and no longer does. There is no **Retry**. | Ask your host. |
| "You’re no longer a member of {workspace}." | The host removed you or this computer. There is no **Retry**, and Crew does not reconnect by itself. | Ask your host. |
| "Live updates for {workspace} stopped because Crew couldn’t confirm the request came from you." | The background service could not confirm that the request came from you. | Choose **Retry**. If it continues, quit and reopen Biorouter. |
| "Too many Crew windows are showing live updates for {workspace}. Close one, then retry." | Too many Biorouter windows show this workspace. | Close one Crew window, then choose **Retry**. |
| "An update from {workspace} was too large to show." | One update was over the size Crew can show. | Choose **Retry**. |

One of these sentences may follow a live update message. They say what happened to a message you had started but not sent:

- "Your unsent draft is retained."
- "Access or privacy changed, so your unsent draft was cleared."
- "Workspace privacy or selected channel access changed, so your unsent draft was cleared."
- "Your unsent draft for it was cleared."

### Messages in the Sign in window

These appear in an error note under the terminal box, except the first, which appears inside the box.

| Message | Cause | What to do |
|---|---|---|
| "Authentication refused. Close and reconnect." | You signed in, but Crew could not start on the server, or the background service refused to finish signing in. | Choose **Close**, then **Reconnect** in the workspace menu. If the "Crew isn’t set up" screen appears, see [Crew is not set up on the server](#crew-is-not-set-up-on-the-server). |
| "SSH authentication ended (exit {code}). Choose Reconnect to check the connection." | Signing in ended before Crew confirmed it. A wrong password or code, or a server that closed the connection, ends it this way. {code} reads "unknown" when there was no code. | Choose **Close**. Then choose **Reconnect** in the workspace menu and sign in again. |
| "Your input couldn’t reach the server. Close this sign-in and try again." | What you typed did not reach the server. | Choose **Close**, then **Reconnect**. |
| "Signing in needs the Biorouter desktop app." | You opened Crew outside the desktop app, for example in a web browser. | Sign in from the Biorouter desktop app, or use `biorouter crew auth` in a terminal. |
| "Invalid Crew connection ID." | The window was opened for a connection this computer does not have. | Choose **Close**. Reopen Crew and try again. |
| "The local daemon is not available." | The background service is not running for this window. | Quit and reopen Biorouter. |
| "SSH authentication is already opening." | A second Sign in window was requested while one was starting. | Choose **Close**, then open the Sign in window again, once. |
| "Close an existing terminal before opening SSH authentication." | Too many terminals are open in this window. | Close a terminal you do not need, then try again. |
| "The authentication window or connection closed." | The window or the connection closed while signing in was starting. | Choose **Reconnect** and sign in again. |
| "SSH authentication failed." | Signing in failed for a reason with no other description. | Choose **Close**, then **Reconnect**. |
| "SSH authentication could not start." | The background service could not prepare signing in. It may show its own reason instead. | Choose **Close**. Check **Connection settings…**, then choose **Reconnect**. |
| "Invalid daemon authentication session." | The background service gave an answer the app could not use. | Choose **Close**, then quit and reopen Biorouter. |
| "Could not attach to daemon SSH authentication." | The app could not connect to the session for signing in. | Choose **Close**, then **Reconnect**. |
| "Daemon SSH authentication closed." | The session for signing in closed. | Choose **Close**, then **Reconnect**. |
| "Authentication terminal unavailable; close and reconnect." | The terminal box closed before your typing reached it. | Choose **Close**, then **Reconnect**. |
| "Authentication input exceeds frame limit." | You pasted too much text at once. | Type your answer instead of pasting a long text. |

### Last error in the workspace menu

When the workspace is not connected, the workspace menu shows the last error. Besides the connection bar messages above, it can show these:

| Message | Cause | What to do |
|---|---|---|
| "The connection to this workspace dropped while it was idle." | The connection dropped while nobody was using it. | Wait. Crew reconnects by itself after a network problem. If it does not, choose **Reconnect**. |
| "SSH bridge failed. Reconnect; inspect any submitted operation before retrying because its outcome may be unknown." | The connection broke while it carried a request. | Choose **Reconnect**. Check whether your last message or action went through before you repeat it. |
| "This computer is no longer a member of {workspace}." | The host removed you or this computer. Crew does not reconnect by itself. | Ask your host. |
| "Signed in, but Crew couldn't start on the server. Crew may not be set up for your account there." | Signing in worked, but Crew did not start on the server. | Choose **Reconnect**. If the "Crew isn’t set up" screen appears, follow it. |

### Joining and the connection

These notes appear on the join screen while you wait for your host.

| Message | Cause | What to do |
|---|---|---|
| "Reconnecting to {workspace}…" | The connection dropped, and Crew is reconnecting once by itself. | Wait. |
| "Crew isn’t connected to {workspace}, so it can’t check your invitation." | The automatic reconnect did not work. | Choose **Reconnect**. Your code does not change. |
| "Crew couldn’t check your invitation. It tries again by itself." | Checking the invitation failed once. | Wait. Crew checks again every 5 seconds while the window is visible. |
| "Joining didn’t finish." | Your host let you in, but the last step failed. | Choose **Try again**. |

For invitation messages, see [Joining a workspace](joining-a-workspace.md).

### Connection settings refusals

These appear inside the Connection settings window when a save is refused.

| Message | Cause | What to do |
|---|---|---|
| "Choose this private SSH connection's institution before saving" | The connection is Private and has no institution. | Enter the institution ID, such as `ucsf`. |
| A message that starts with "Crew institution must be a canonical institution ID" | The institution ID has the wrong form. | Use lowercase letters, numbers, `-` or `_`, such as `ucsf`. |
| "Crew aliases have different institutions; use a separately verified cluster connection" | Another workspace on the same server uses a different institution. | Use the same institution as the other workspace on that server, or ask your host. |
| "Identity file must be an absolute path" | The **Identity file** field holds a path that does not start with `/`, for example one that starts with `~`. | Enter the full path, such as `/Users/you/.ssh/id_ed25519`, or leave the field empty. |
| "Crew settings are being saved by another Biorouter process. Try again in a moment." | Another Biorouter window or command is saving Crew settings at the same time. | Wait a few seconds, then save again. |

### Background service messages

These can appear in the connection bar or in a dialog after any action.

| Message | Cause | What to do |
|---|---|---|
| "This feature needs a newer Biorouter background service. Quit and reopen Biorouter." | The background service is older than the app. | Quit Biorouter and open it again. |
| "Biorouter sent {something} that Crew couldn't read. Retry, or quit and reopen Biorouter." | The background service gave an unexpected answer. | Try the action again. If it repeats, quit and reopen Biorouter. |
| "Crew could not complete that action." | An action failed without a reason. | Try the action again. |
| "Crew couldn’t complete that action." | An action in a dialog failed without a reason. | Try the action again. |
| "Authorize this action in the Crew panel or native Crew CLI with your human approval secret. Agent tools use their separate task grant." | The request did not carry proof that a person made it. | Do the action yourself in Crew, or in the `biorouter crew` command. |
| "This daemon cannot verify human Crew actions. Start the trusted desktop launcher or biorouter crew daemon start with your separately held approval secret." | The background service started without the key that proves your actions are yours. | Quit Biorouter and open it again from its usual icon. See [Administration](administration.md) if it continues. |
| "Crew is still checking this workspace’s privacy. Try again in a moment." | Crew has not confirmed the workspace's privacy yet. | Wait until the status row shows Connected, then try again. |
| "Refresh the workspace to verify connection privacy before sending." | Crew has not confirmed the workspace's privacy, so it holds your message. | Wait until the status row shows Connected, then send. |
| "Refresh the workspace to verify connection privacy before granting agent access." | The same, for agent access. | Wait until the status row shows Connected, then try again. |

## Related documentation

- [Crew user manual](README.md): the list of every page in this manual.
- [Getting started](getting-started.md): what you need before you use Crew, and where the status row, workspace menu and connection bar are.
- [Joining a workspace](joining-a-workspace.md): the invitation, your code, and the join screen.
- [Hosting a workspace](hosting-a-workspace.md): what the host does when a member sees "Crew isn’t set up".
- [Privacy and security](privacy-and-security.md): Private and Public, institutions, keys and fingerprints.
- [Agents and chat access](agents-and-chat-access.md): chat access to Crew channels, and the remote work folder.
- [Command line](command-line.md): the `biorouter crew` commands, including `status`, `auth` and `connect`.
- [Administration](administration.md): installing `biorouter-crew` in each account, and the background service.
- [SSH hop policy](../research/biorouter-crew/ssh-hop-policy.md): known hosts setup and jump host rules, written for IT staff.
