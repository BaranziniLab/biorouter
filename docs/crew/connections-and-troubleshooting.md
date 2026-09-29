# Connections and troubleshooting

> **What this is.** How Crew connects to your lab's workspace, how to sign in and change connection settings, and what each status and error message asks you to do.
> **Status:** Current.
> **Audience:** Lab members who use Crew in the Biorouter desktop app. No SSH or terminal experience is needed.

Your lab's workspace runs on a shared lab server. Your computer reaches it over SSH, the secure login that lab servers use. The Biorouter background service on your computer keeps the connection open and checks the server's identity.

A word in braces stands for a name that Crew fills in. For example, "Connect to {workspace}" appears as "Connect to chen-lab". {server} is the server's name in your SSH settings file (`~/.ssh/config`), or its address if that file has no entry. {server address} is always the address.

## What each status means

| Status | What it means and what to do |
|---|---|
| "Connected" | You are connected, and Crew verified the workspace. |
| "Connecting…", "Checking connection", "Updating…" or "Reconnecting…" | Wait. If the Sign in window is open, sign in. |
| "Updates unavailable" | Live updates stopped. Choose **Retry** in the connection bar. |
| **Sign-in needed** | Choose the word to [sign in](#sign-in-to-the-server). |
| "Can’t verify server" | See [Server identity checks](#server-identity-checks). |
| "Not set up on this server" | See [Crew missing from your server account](#crew-missing-from-your-server-account). |
| "Not joined yet" | Your host has not let you in yet. See [Joining a workspace](joining-a-workspace.md). |
| "Offline" | You are not connected. This follows a computer restart, **Disconnect**, a saved change to your server login, port, identity file or jump hosts in **Connection settings…**, or a dropped connection. After a drop, Crew [reconnects by itself](#automatic-reconnection). Otherwise, choose **Connect to {workspace}**. |
| "Connected" with "The workspace server has stopped saving changes. Reading still works." | The server's disk is full or failing. You can read, but posts, uploads and other changes are refused. See [A server that stopped saving](#a-server-that-stopped-saving). |
| "Can’t connect" | The last attempt failed. The main area says why, such as "Crew isn’t running on {server}" or "{server} refused this computer’s SSH key". See [Messages and what to do](#messages-and-what-to-do). |

## Connect and disconnect

### Connect to a workspace

1. In Biorouter, choose **Crew** in the left sidebar. With several workspaces, choose the workspace name, then one under **Switch workspace**.
2. In the main area, choose **Connect to {workspace}**.
3. If the Sign in window opens, [sign in](#sign-in-to-the-server).

The status row shows "Connected", and your channels appear. A failed attempt adds "Tried again at {time}. {reason}." under the button. **Reconnect**, **Try again**, **Connect now** and **Connect in Crew** start the same attempt. From a terminal, see [Sign in and stay connected](command-line.md#sign-in-and-stay-connected).

### When you reopen Biorouter

On a Mac or a Linux computer, quitting Biorouter leaves the background service running and the workspace connected. When you reopen Biorouter, it connects to the service without asking you anything ([The background service](getting-started.md#the-background-service)). A pending join shows the same code, or finishes by itself.

After your computer restarts, and on Windows after every quit, the service stops. To connect again, open Biorouter and choose **Connect to {workspace}**. The status row shows "Connected".

### Disconnect from a workspace

Choose the workspace name, then **Disconnect**. Crew does not ask you to confirm. The status changes to "Offline", and Crew does not reconnect by itself. A pending join keeps its code.

### Connection tools in the workspace menu

The workspace menu shows who you are signed in as, such as "Signed in as @bob on lab.example.edu", once Crew has verified the connection. Before that, and while you are offline, it shows only "Server lab.example.edu". It also shows the status and the workspace fingerprint with **Copy**. When you are not connected, the last error replaces the fingerprint. Point at a line cut short to read all of it. With the keyboard, press the Up arrow from the first menu item to reach **Copy**.

**Reconnect** appears when you are not connected, and **Sign in…** only at **Sign-in needed**. **Disconnect** and **Connection settings…** are always there. An unavailable item says why: "Available after you join", "Available once the connection is verified" or "Available once you’re connected". With two or more workspaces, **Switch workspace** lists them, each with a status dot.

## Automatic reconnection

After a network drop, or while the workspace is stopped on the server, the background service reconnects by itself for up to an hour, and your channels come back without a click. It never asks for a password or a code, and never opens the Sign in window. To try at once, choose **Connect to {workspace}**.

- After a drop, it tries again after 20 seconds, then 1 minute and 3 minutes later, then every 5 minutes. Meanwhile the main area says "Reconnecting to {workspace}" and "The connection dropped, and Crew has been dialling it again by itself since {time}. That can take a few minutes, and nothing reaches the workspace until then. You can connect now instead.", with **Connect now**.
- While the workspace is stopped on the server, it tries every 30 seconds, so it connects within about a minute of the host starting Crew. A member reads "The workspace server isn’t running. Once {host} starts Crew, this computer connects by itself within a few minutes, or you can connect now."

Crew does not reconnect by itself in these cases. Choose **Connect to {workspace}**, or follow the screen in the main area.

- **Disconnect**, or a saved change to your server login, port, identity file or jump hosts in **Connection settings…**.
- A network outage longer than an hour.
- A server that wants a password or a code. If yours always asks, you sign in after every drop.
- A server that refused this computer's SSH key.
- A server Crew cannot verify, or where Crew is not set up.
- Removal from the workspace.
- A stopped background service. See [When you reopen Biorouter](#when-you-reopen-biorouter).

A chat with Crew access shows "Crew is offline." with **Connect now** or **Connect in Crew**. Both open Crew and connect. See [When Crew goes offline](agents-and-chat-access.md#when-crew-goes-offline).

## Sign in to the server

Some lab servers ask for a password, a verification code, or both. A verification code is the short code your institution's login system gives you each time. The Sign in window opens by itself when you connect and the server asks. You can also open it from **Sign-in needed**, **Sign in…** in the workspace menu, or **Sign in** in the main area.

1. In the window "Sign in to {server address}", wait for the server's question.
2. Type your password and press Enter. The characters do not appear.
3. If the server asks for a verification code, type it and press Enter.
4. If the server lists numbered options, such as "Passcode or option (1-3):", type a number and press Enter. Approve any request sent to your phone.

The window closes when the server accepts you. To stop, choose **Close**. Escape does nothing.

**Trouble signing in?** under the box has tips about your login and [jump hosts](#connection-settings).

The Sign in window asks for the server's password even when your SSH settings say `BatchMode yes`. A jump host that wants a password or a code cannot ask through it while `BatchMode yes` applies to that jump host, so ask IT to leave `BatchMode` out of its settings.

## Server identity checks

Every server has a host key that proves its identity. Your computer keeps the keys it trusts in its known hosts file, `~/.ssh/known_hosts`. Crew connects only to a server whose key is there, and checks that the workspace key matches your invitation. Crew never adds a key for you. A failed check shows "Can’t verify server" and one of three screens.

### An unverified server

"Can’t verify {server address} yet" means the server's key is not in your known hosts file. This is normal the first time on each computer. When the key that is missing belongs to a jump host, the screen names the jump host instead, and that is the one to verify.

1. Ask your IT team for the server's fingerprint. It starts with `SHA256:`.
2. Follow [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer). **How do I verify it?** offers **Open a terminal here**. If terminals are new to you, ask IT to help.
3. Choose **Try again**. Crew connects, and the Sign in window opens if the server asks.

### A changed server identity

"{server address}’s identity changed" means the server was rebuilt, or another machine is answering in its place. Treat it as a security problem until IT says otherwise.

1. Choose **Copy details for IT**, and send the details to IT. Do not connect until IT confirms the change.
2. Remove the old key, or ask IT to. In a terminal, type `ssh-keygen -R lab.example.edu` with your server's address, or `ssh-keygen -R '[lab.example.edu]:2222'` for another port.
3. Choose the workspace name, then **Reconnect**. Expect "Can’t verify {server address} yet".
4. Follow [An unverified server](#an-unverified-server) with the new fingerprint.

### A different workspace

"This isn’t the workspace you joined" means the server answered with a different workspace key. **Connection settings…** on that screen shows your server.

1. Choose **Copy details**. "Copied" appears beside the button.
2. Paste the details into a message to your host, and send it.
3. Wait for your host to explain what changed. If the workspace was created again, your host sends you a new invitation. Otherwise, when your host says the server is fixed, choose **Reconnect** in the workspace menu. The status row reads "Connected".

## Crew missing from your server account

"Crew isn’t set up for your account on {server address}" in the main area means your own server account has no `~/.local/bin/biorouter-crew`. [Install Crew in your server account](joining-a-workspace.md#install-crew-in-your-server-account) says who can install it and how.

1. Install it yourself, or copy the ready message on the screen and send it to IT, or to your host to pass on.
2. When it is installed, choose **Try again**. Crew connects.

## Connection settings

Choose the workspace name, then **Connection settings…**.

| Field | What to enter |
|---|---|
| **Connection name** | The name this computer shows. |
| **Your server login** | Such as `you@server.example.edu`. Crew fills it in when you join. Change it only if IT tells you to. |
| **Privacy** | **Private** or **Public**. See [Change your connection's privacy](privacy-and-security.md#change-your-connections-privacy). |
| **Institution** | Required for Private. A short ID, such as `ucsf`. |
| **Port** | The SSH port. The default is 22. |
| **Identity file** | The full path of your SSH key, such as `/Users/you/.ssh/id_ed25519`. A path starting with `~` is refused. Leave it empty to use your SSH settings. |
| **Jump hosts** | A gateway server from IT that you pass through first. It also needs settings in your SSH settings file. See [SSH requirements](administration.md#ssh-requirements). |
| **Remote work folder**, **Let my agent run commands in this folder** | See [Agents and chat access](agents-and-chat-access.md). |

**Port** and the fields below it are under **Advanced**. A wrong value gets a note under its field. **Workspace details** copies IDs that support staff may ask for.

Choose **Save connection**. A saved change ends the Crew access of chats on that server. In such a chat, choose **Grant access again**, or start a new chat.

- A change to **Privacy**, **Institution**, **Connection name** or the remote work folder keeps the route to the server. If the workspace was connected, Crew reconnects at once.
- A change to **Your server login**, **Port**, **Identity file** or **Jump hosts** disconnects the workspace. Choose **Connect to {workspace}** afterwards.

A value Crew refuses gets a note under its field, which opens **Advanced** if it is there. An institution that does not match the workspace's gets, for example, "This connection is for stanford, but chen-lab belongs to ucsf. Use ucsf here."

Workspaces on one server share one privacy setting and one institution. If any is Private, all stay Private.

To remove a workspace, choose **Remove {workspace} from this computer…**, then **Remove**. This disconnects it, ends every chat's access to it, and deletes this computer's key for it. Your messages stay on the server, and you stay a member, so your old invitation does not bring the workspace back. To use it here again, follow [Add this computer to your existing account](joining-a-workspace.md#add-this-computer-to-your-existing-account).

If you are the host, read [Limits of the host role](hosting-a-workspace.md#limits-of-the-host-role) before you remove a workspace.

## A server that stopped saving

When the lab server's disk is full or failing, the workspace keeps running but saves no more changes. Once your Crew learns of it, when it next connects to the workspace or a change is refused, the status row still reads "Connected", and the connection bar says "The workspace server has stopped saving changes. Reading still works." Reading channels and files works. Posts, uploads and other changes are refused, and an upload that was running pauses with "The workspace server couldn’t save it". The app updates the bar when its window comes to the front, so if Biorouter stayed in front the whole time, switch to another window and back to see where the server stands now.

- If you host the workspace, the bar adds "Free space on the server, then restart Crew there." Follow [Server storage full or failing](administration.md#server-storage-full-or-failing). The workspace saves changes again only after the restart.
- Otherwise, the bar names your host, such as "Ask Alice Chen (@alice) to free space on the server and restart Crew." Nothing you do on your computer helps. Once the host restarts Crew, Crew connects again by itself, and you can resume paused uploads.

From a terminal, `biorouter crew status` and `connections show` print the same sentence, and for the host what to run on the server.

## Messages and what to do

For messages on the join screen, see [Problems while you wait](joining-a-workspace.md#problems-while-you-wait).

### Connection messages

| Message | What to do |
|---|---|
| "Can’t reach…" or "Couldn’t reach…" | Check that you are online, and on the VPN (virtual private network) if your institution requires one. Crew keeps trying for an hour. |
| "Crew isn’t running on {server}" | The workspace is stopped on the server, for example after the server restarted. If you host it, the screen shows the line that starts it, with **Copy**: run it on the server, as [After the server restarts](hosting-a-workspace.md#after-the-server-restarts) shows. Otherwise, the screen says "The workspace server isn’t running. Once {host} starts Crew, this computer connects by itself within a few minutes, or you can connect now." Ask your host to start Crew if they have not. Crew tries every 30 seconds, so it connects within about a minute of the start. |
| "{server} refused this computer’s SSH key" | The server offered no password or code, so the Sign in window cannot help. Check **Your server login** and **Identity file** in **Connection settings…**, and ask IT which login and key to use. |
| "Can’t connect…", "Crew can’t connect." or "It didn’t connect" | Your server name may reach a different machine from the one that runs Crew. Ask your host. Otherwise, check **Connection settings…** with IT. |
| "…asked you to sign in" | [Sign in](#sign-in-to-the-server). |
| "Crew couldn’t verify…" | See [Server identity checks](#server-identity-checks). |
| "Crew isn’t running for you…" or "Signed in, but Crew couldn't start…" | See [Crew missing from your server account](#crew-missing-from-your-server-account). |
| "Crew SSH host…", such as "Crew SSH host gateway (the jump host on this connection's route) requires StrictHostKeyChecking yes…" | Your SSH settings file lacks a setting that Crew requires for that jump host. Send the sentence to IT with a link to [SSH requirements](administration.md#ssh-requirements), then connect again. If it adds "This computer is the workspace's server, so it needs no jump host", clear **Jump hosts** in **Connection settings…** instead ([Join from the workspace's own server](command-line.md#join-from-the-workspaces-own-server)). |
| "You already use this server for {other workspace}…" | One computer uses one institution for each server. Ask your host which is right. |
| "…dropped while it was idle." | Wait, or choose **Reconnect**. |
| "SSH bridge failed…" | The connection broke during a request, and Crew does not resend it. Choose **Reconnect**. Before you repeat your last action, check if it went through. |
| "Reconnecting to {workspace}. Nothing was sent; try again in a moment." or "Reconnecting to {workspace} in about {N} seconds. Nothing was sent. Connect now to try at once." | Crew is dialling a broken connection again, and your action was not sent. Try it again once the status reads "Connected", or choose **Connect now** in the main area to try at once. |
| "Crew couldn’t confirm whether this reached {workspace}…" | The connection broke after the request left. Check the channel before you repeat the action. |
| "…no longer a member…" or "…doesn’t recognize this computer any more…" | Your host removed you or this computer. Ask your host. |
| "{workspace} was removed from this computer, so it can’t be opened." | The workspace was removed here while Crew was open, for example with `biorouter crew connections remove`, and Crew opened another. To use it here again, follow [Add this computer to your existing account](joining-a-workspace.md#add-this-computer-to-your-existing-account). |
| "Your Crew vault is locked." | Choose **Unlock**, and enter your vault passphrase. If it is refused, choose **Unlock** again. See [Use an encrypted vault](privacy-and-security.md#use-an-encrypted-vault). |
| "A new device was added to your account…" | Choose **Review**. If you did not add that device, tell your host. |

### Live update messages

**Retry** in the connection bar checks the connection first.

| Message | What to do |
|---|---|
| "Biorouter lost its connection to its background service, so Crew can’t reach your workspaces until Biorouter reconnects." with **Reconnect** | Biorouter reconnects by itself. If the message stays, choose **Reconnect**, the same button as the notice in the app's sidebar. Crew then loads your workspaces again. |
| "Live updates keep stopping…" or "…couldn’t confirm the request came from you." | Choose **Retry**. If it repeats, quit and reopen Biorouter. |
| "Your access to {workspace} changed." | Choose **Retry**. If it repeats, ask your host. |
| "You no longer have access to {channel}…" | Ask the channel owner if you need access again. |
| "…doesn’t recognize this computer yet." | This is normal while you wait for your host. |
| Any other message, such as "Live updates…stopped." or "Earlier messages couldn’t be loaded." | Choose **Retry**. |

### Sign in window messages

Here, **Reconnect** means: choose **Close**, then **Reconnect** in the workspace menu.

| Message | What to do |
|---|---|
| "SSH authentication ended (exit {code})…" | If you typed a password or code, it was probably wrong: choose **Close**, then **Connect to {workspace}**, and sign in again. If no question ever appeared (exit 255 is common), the server or a jump host refused this computer without asking, for example a key the server does not accept, a jump host whose key is not verified, or `BatchMode yes` on a jump host. Check the login and jump hosts in **Connection settings…** with IT. |
| "Authentication refused. Close and reconnect." | **Reconnect**. If "Crew isn’t set up" appears, see [Crew missing from your server account](#crew-missing-from-your-server-account). |
| "SSH authentication is already opening." | Choose **Close**, then open the Sign in window once. |
| "Authentication input exceeds frame limit." | Type your answer instead of pasting it. |
| "The local daemon is not available." or "Invalid daemon authentication session." | Quit and reopen Biorouter. |
| Any other message, such as "Your input couldn’t reach the server…" | **Reconnect**. If it repeats, check **Connection settings…**. |

### Other messages

| Message | What to do |
|---|---|
| "Crew needs the Biorouter desktop app" or "Crew isn't available in a browser opened with biorouter serve…" | You opened Crew in a web browser through `biorouter serve`, where Crew does not work. Open Crew in the desktop app on your own computer, or use `biorouter crew` in a terminal there. Signing in again or restarting `biorouter serve` does not help ([Getting started](getting-started.md)). |
| "Choose this private SSH connection's institution…" or "…canonical institution ID…" | Enter a short ID in lowercase letters, numbers, `-` or `_`, such as `ucsf`. |
| "This connection is for {institution}, but {workspace} belongs to {institution}…", "…is also saved on this computer for the same workspace, under institution…" or "…one computer can't mix institutions on the same server." | Use the workspace's institution in **Connection settings…**, for every workspace on that server, or ask your host. An older background service says "Crew aliases have different institutions…", and an older server "privacy_denied: connection and workspace institutions differ". |
| "Use the key file’s full path…" under **Identity file** in **Connection settings…**, or "Choose the identity file by its full path." or "Identity file must be an absolute path" when you join | Type the key file's full path, such as `/Users/you/.ssh/id_ed25519` on a Mac or `/home/you/.ssh/id_ed25519` on Linux. A path that starts with `~` is not accepted. Or leave the field empty to sign in with your own SSH settings. |
| "…needs a newer Biorouter background service…" | See [Replace an old background service](#replace-an-old-background-service). |
| "Biorouter couldn't reconnect to its background service." with **Try again** and **Quit and reopen** | The background service stopped or was replaced while Biorouter was open, for example by `biorouter crew daemon stop`, and Biorouter could not reconnect by itself. Choose **Try again**. If it repeats, choose **Quit and reopen**. Nothing you were writing is lost. |
| "This computer has no keyring service Biorouter can use…" or "This computer's keyring service isn't answering…" | This Linux computer has no working keyring. Before your first workspace, set up [an encrypted vault](privacy-and-security.md#use-an-encrypted-vault), or run `biorouter crew credentials init` ([Keep device keys in an encrypted vault](command-line.md#keep-device-keys-in-an-encrypted-vault)). If you already joined, start the keyring again, for example by signing in to the computer's desktop. |
| "…didn't let Biorouter use this computer's Crew keys…" | Unlock your keyring, or allow access when it asks, then try again. |
| "The workspace server can’t save messages right now…" | The server's disk is full or failing. If you host the workspace, follow [Server storage full or failing](administration.md#server-storage-full-or-failing). Otherwise, ask your host. |
| "This daemon cannot verify human Crew actions…" | See [Replace an old background service](#replace-an-old-background-service). |
| "…that Crew couldn't read…" | Choose **Retry**. If it repeats, see [Replace an old background service](#replace-an-old-background-service). |
| "Crew could not complete that action." | Try the action again. |
| "Authorize this action in the Crew panel…" | Do the action yourself in Crew, or with `biorouter crew`. |
| "…still checking this workspace’s privacy…" or "Refresh the workspace to verify connection privacy…" | Wait for "Connected", then try again. If the status is "Offline", connect first. |

### Replace an old background service

After a Biorouter update, the background service that was already running is still the old version until Biorouter replaces it. These messages mean the old service is still running:

- "This feature needs a newer Biorouter background service…"
- "Start it for me needs a newer Biorouter background service…"
- "This daemon cannot verify human Crew actions…"

To replace the service on a Mac or Linux computer, quit Biorouter and open it again. Biorouter stops the old service and starts a new one, and the action that showed the message then works. Replacing the service stops running agent tasks and file transfers, so check them afterward.

If you use only the `biorouter` command, run `biorouter crew daemon stop`, then run your command again. It starts a new service.

On Windows, quitting Biorouter stops the service, so quit Biorouter and open it again.

## Related documentation

- [Crew user manual](README.md): every page.
- [Getting started](getting-started.md): the background service and a tour of the Crew view.
- [Joining a workspace](joining-a-workspace.md): verifying the server.
- [Hosting a workspace](hosting-a-workspace.md): restarting Crew on the server.
- [Privacy and security](privacy-and-security.md): privacy, keys and the vault.
- [Agents and chat access](agents-and-chat-access.md): chat access.
- [Command line](command-line.md): every `biorouter crew` command.
- [Administration](administration.md): installing `biorouter-crew` and SSH settings.
