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
| "Offline" | You are not connected. This follows a computer restart, **Disconnect**, a saved change in **Connection settings…**, or a dropped connection. After a drop, Crew [reconnects by itself](#automatic-reconnection). Otherwise, choose **Connect to {workspace}**. |
| "Can’t connect" | The last attempt failed. See [Messages and what to do](#messages-and-what-to-do). |

## Connect and disconnect

### Connect to a workspace

1. In Biorouter, choose **Crew** in the left sidebar. With several workspaces, choose the workspace name, then one under **Switch workspace**.
2. In the main area, choose **Connect to {workspace}**.
3. If the Sign in window opens, [sign in](#sign-in-to-the-server).

The status row shows "Connected", and your channels appear. A failed attempt adds "Tried again at {time}. {reason}." under the button. **Reconnect**, **Try again**, **Connect now** and **Connect in Crew** start the same attempt. From a terminal, see [Sign in and stay connected](command-line.md#sign-in-and-stay-connected).

### When you reopen Biorouter

On a Mac or a Linux computer, quitting Biorouter leaves the background service running and the workspace connected. When you reopen Biorouter, type your approval secret in the window "Connect to existing BioRouter daemon". See [The approval secret](getting-started.md#the-approval-secret). A pending join shows the same code, or finishes by itself.

After your computer restarts, and on Windows after every quit, the service stops. Reopen Biorouter, set an approval secret on a Mac or a Linux computer, and choose **Connect to {workspace}**.

### Disconnect from a workspace

Choose the workspace name, then **Disconnect**. Crew does not ask you to confirm. The status changes to "Offline", and Crew does not reconnect by itself. A pending join keeps its code.

### Connection tools in the workspace menu

The workspace menu shows who you are signed in as, the status, and the workspace fingerprint with **Copy**. When you are not connected, the last error replaces the fingerprint. With the keyboard, press the Up arrow from the first menu item to reach **Copy**.

**Reconnect** appears when you are not connected, and **Sign in…** only at **Sign-in needed**. **Disconnect** and **Connection settings…** are always there. An unavailable item says why: "Available after you join", "Available once the connection is verified" or "Available once you’re connected". With two or more workspaces, **Switch workspace** lists them, each with a status dot.

## Automatic reconnection

After a network drop, the background service reconnects by itself for up to an hour, and your channels come back without a click. It never asks for a password or a code, and never opens the Sign in window. To try at once, choose **Connect to {workspace}**.

Crew does not reconnect by itself in these cases. Choose **Connect to {workspace}**, or follow the screen in the main area.

- **Disconnect**, or a saved change in **Connection settings…**.
- A network outage longer than an hour.
- A server that wants a password or a code. If yours always asks, you sign in after every drop.
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

## Server identity checks

Every server has a host key that proves its identity. Your computer keeps the keys it trusts in its known hosts file, `~/.ssh/known_hosts`. Crew connects only to a server whose key is there, and checks that the workspace key matches your invitation. Crew never adds a key for you. A failed check shows "Can’t verify server" and one of three screens.

### An unverified server

"Can’t verify {server address} yet" means the server's key is not in your known hosts file. This is normal the first time on each computer.

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

Choose **Save connection**. If you changed something, Crew disconnects the workspace, so choose **Connect to {workspace}** afterwards. Chats with Crew access on that server lose it. In such a chat, choose **Grant access again**, or start a new chat.

Workspaces on one server share one privacy setting and one institution. If any is Private, all stay Private.

To remove a workspace, choose **Remove {workspace} from this computer…**, then **Remove**. This disconnects it, ends every chat's access to it, and deletes this computer's key for it. Your messages stay on the server, and you stay a member, so your old invitation does not bring the workspace back. To use it here again, follow [Add this computer to your existing account](joining-a-workspace.md#add-this-computer-to-your-existing-account).

If you are the host, read [Limits of the host role](hosting-a-workspace.md#limits-of-the-host-role) before you remove a workspace.

## Messages and what to do

For messages on the join screen, see [Problems while you wait](joining-a-workspace.md#problems-while-you-wait).

### Connection messages

| Message | What to do |
|---|---|
| "Can’t reach…" or "Couldn’t reach…" | Check that you are online, and on the VPN (virtual private network) if your institution requires one. Crew keeps trying for an hour. |
| "Can’t connect…", "Crew can’t connect." or "It didn’t connect" | If it worked before, Crew may have [stopped on the server](hosting-a-workspace.md#after-the-server-restarts). Your server name may also reach a different machine from the one that runs Crew. Ask your host. Otherwise, check **Connection settings…** with IT. |
| "…asked you to sign in" | [Sign in](#sign-in-to-the-server). |
| "Crew couldn’t verify…" | See [Server identity checks](#server-identity-checks). |
| "Crew isn’t running for you…" or "Signed in, but Crew couldn't start…" | See [Crew missing from your server account](#crew-missing-from-your-server-account). |
| "Crew SSH host…", such as "Crew SSH host gateway requires StrictHostKeyChecking yes…" | Your SSH settings file lacks a setting that Crew requires. Send the sentence to IT with a link to [SSH requirements](administration.md#ssh-requirements), then connect again. |
| "You already use this server for {other workspace}…" | One computer uses one institution for each server. Ask your host which is right. |
| "…dropped while it was idle." | Wait, or choose **Reconnect**. |
| "SSH bridge failed…" | The connection broke during a request, and Crew does not resend it. Choose **Reconnect**. Before you repeat your last action, check if it went through. |
| "…no longer a member…" or "…doesn’t recognize this computer any more…" | Your host removed you or this computer. Ask your host. |
| "Your Crew vault is locked." | Choose **Unlock**, and enter your vault passphrase, not your approval secret. If it is refused, choose **Unlock** again. See [Use an encrypted vault](privacy-and-security.md#use-an-encrypted-vault). |
| "A new device was added to your account…" | Choose **Review**. If you did not add that device, tell your host. |

### Live update messages

**Retry** in the connection bar checks the connection first.

| Message | What to do |
|---|---|
| "Live updates keep stopping…" or "…couldn’t confirm the request came from you." | Choose **Retry**. If it repeats, quit and reopen Biorouter. |
| "Your access to {workspace} changed." | Choose **Retry**. If it repeats, ask your host. |
| "You no longer have access to {channel}…" | Ask the channel owner if you need access again. |
| "…doesn’t recognize this computer yet." | This is normal while you wait for your host. |
| Any other message, such as "Live updates…stopped." or "Earlier messages couldn’t be loaded." | Choose **Retry**. |

### Sign in window messages

Here, **Reconnect** means: choose **Close**, then **Reconnect** in the workspace menu.

| Message | What to do |
|---|---|
| "SSH authentication ended (exit {code})…" | Often a wrong password or code. **Reconnect**, and sign in again. |
| "Authentication refused. Close and reconnect." | **Reconnect**. If "Crew isn’t set up" appears, see [Crew missing from your server account](#crew-missing-from-your-server-account). |
| "SSH authentication is already opening." | Choose **Close**, then open the Sign in window once. |
| "Authentication input exceeds frame limit." | Type your answer instead of pasting it. |
| "The local daemon is not available." or "Invalid daemon authentication session." | Quit and reopen Biorouter. |
| "Signing in needs the Biorouter desktop app." | Use the desktop app, or run `biorouter crew auth`. |
| Any other message, such as "Your input couldn’t reach the server…" | **Reconnect**. If it repeats, check **Connection settings…**. |

### Other messages

| Message | What to do |
|---|---|
| "Choose this private SSH connection's institution…" or "…canonical institution ID…" | Enter a short ID in lowercase letters, numbers, `-` or `_`, such as `ucsf`. |
| "Crew aliases have different institutions…", "privacy_denied: connection and workspace institutions differ" or "…one computer can't mix institutions on the same server." | Use the workspace's institution in **Connection settings…**, for every workspace on that server, or ask your host. |
| "Identity file must be an absolute path" | Enter a path that starts with `/`, or leave the field empty. |
| "…needs a newer Biorouter background service…" or "This daemon cannot verify human Crew actions…" | See [Replace an old background service](#replace-an-old-background-service). |
| "…that Crew couldn't read…" | Choose **Retry**. If it repeats, see [Replace an old background service](#replace-an-old-background-service). |
| "Crew could not complete that action." | Try the action again. |
| "Authorize this action in the Crew panel…" | Do the action yourself in Crew, or with `biorouter crew`. |
| "…still checking this workspace’s privacy…" or "Refresh the workspace to verify connection privacy…" | Wait for "Connected", then try again. If the status is "Offline", connect first. |

### Replace an old background service

After a Biorouter update, the background service that was already running stays the old version. On a Mac or Linux computer, quitting and reopening Biorouter does not replace it, because Biorouter connects to the service that is still running. So these messages stay, even though some of them say to quit and reopen Biorouter:

- "This feature needs a newer Biorouter background service…"
- "Start it for me needs a newer Biorouter background service…"
- "This daemon cannot verify human Crew actions…"

To replace the service on a Mac or Linux computer:

1. Quit Biorouter.
2. Restart your computer.
3. Open Biorouter, and set your approval secret when it asks ([The approval secret](getting-started.md#the-approval-secret)).

The action that showed the message now works. The restart stops running agent tasks and file transfers, so check them afterward.

If you use the `biorouter` command, you can end the service without a restart:

1. Run `biorouter crew daemon stop`.
2. Type your approval secret. The command prints "Biorouter daemon stopped."
3. Open Biorouter. Biorouter asks you to set an approval secret.

After "This daemon cannot verify human Crew actions…" that command is refused, so follow [If you forget the approval secret](command-line.md#if-you-forget-the-approval-secret) instead.

On Windows, quitting Biorouter stops the service, so quit Biorouter and open it again.

## Related documentation

- [Crew user manual](README.md): every page.
- [Getting started](getting-started.md): the approval secret and a tour of the Crew view.
- [Joining a workspace](joining-a-workspace.md): verifying the server.
- [Hosting a workspace](hosting-a-workspace.md): restarting Crew on the server.
- [Privacy and security](privacy-and-security.md): privacy, keys and the vault.
- [Agents and chat access](agents-and-chat-access.md): chat access.
- [Command line](command-line.md): every `biorouter crew` command.
- [Administration](administration.md): installing `biorouter-crew` and SSH settings.
