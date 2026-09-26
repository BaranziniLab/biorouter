# Joining a workspace

> **What this is.** How to join your lab's Crew workspace, and what to do when a step stops you.
> **Status:** Current.
> **Audience:** Lab members who received a Crew invitation. You need no SSH experience.

To join, verify the lab server once, have Crew installed in your server account, paste the invitation from your host (the person who created the workspace), choose Join, and send your host the code Crew shows. When your host enters it, the workspace opens.

Examples use the host Alice Chen (@alice), the workspace `chen-lab` on `lab.example.edu`, and your username `bob`. Braces, such as {team}, mark a name Crew fills in.

## Before you start

| You need | If it is missing |
|---|---|
| The Biorouter desktop app. | See [Installation and setup](../getting-started/installation.md). |
| An approval secret, on a Mac or Linux computer. | See [The approval secret](getting-started.md#the-approval-secret). |
| An account on the lab server, reached with SSH (Secure Shell, how your computer signs in to a server). | Ask your IT team. |
| Your exact server username, sent to your host before they invite you. It is the name before the @ in `ssh bob@lab.example.edu`. `bob` does not match `crew_bob`. | Ask IT for it. |
| The invitation. It lasts 24 hours. | Ask your host to invite you again. |
| The server verified on this computer. | Follow [Verify the server on this computer](#verify-the-server-on-this-computer). |
| `biorouter-crew` in your own account on the server. | See [Install Crew in your server account](#install-crew-in-your-server-account). |

### Verify the server on this computer

Crew connects only to a server whose key is in your known hosts file, `~/.ssh/known_hosts`, and never adds one for you. Do this once per server on each computer.

1. Get the server fingerprint. It starts with `SHA256:`. Your host may have sent it with the invitation. If not, ask IT. If IT gave you a jump host (a server you pass through first), ask IT for its fingerprint too. A server fingerprint is neither the workspace fingerprint nor your code.
2. Open a terminal. On a Mac, press Command and Space, type Terminal, and press Return. Crew's "Can’t verify" screen also offers **How do I verify it?**, then **Open a terminal here**.
3. Type `ssh bob@lab.example.edu` with your username and server, and press Enter.
   - For a port other than 22, add `-p` and the port: `ssh -p 2222 bob@lab.example.edu`.
   - For a jump host, add `-J` and its name: `ssh -J gateway.ucsf.edu bob@lab.example.edu`.
   - If you do not know the server or its port, paste the invitation in the Join dialog (steps 1 to 3 of [Join from the Biorouter app](#join-from-the-biorouter-app)). The server is the name after "on" in the summary. Choose **Advanced**: the grey number in **Port** is the port. Choose **Cancel**. Nothing is saved.
4. Read the answer:
   - "Are you sure you want to continue connecting", after a `SHA256:` value: go to step 5.
   - A password prompt, or the server's prompt, such as `bob@lab:~$`: the server is already verified. Press Ctrl+C at a password prompt, or type `exit` at the server's prompt.
   - "Could not resolve hostname": check the spelling of the server name, or ask IT.
   - "timed out" or "Connection refused": connect to your VPN (the app that connects you to your institution's network), check the port, and try again.
   - "Permission denied": ask IT which login or key to use.
   - "REMOTE HOST IDENTIFICATION HAS CHANGED": stop, and send the whole message to IT. When IT confirms the change, remove the old key (step 2 of [A changed server identity](connections-and-troubleshooting.md#a-changed-server-identity)), and start again at step 3.
5. Compare the `SHA256:` value with the one you got for that host, character by character. With `-J`, SSH asks about the jump host first, then the server. If a value differs, type `no` and tell IT.
6. If it matches, type `yes` and press Enter. SSH prints "Permanently added … to the list of known hosts." If SSH then asks about the next host, repeat step 5.
7. Sign in with your password or code. The terminal does not show a password. Then type `exit`.
8. In Crew, choose **Try again**, or join as below.

Typing `yes` is the step Crew's help calls adding "the full key". Crew may then ask for your password again.

### Install Crew in your server account

Crew needs its server program at exactly `~/.local/bin/biorouter-crew` in your own account on the lab server. Your host's copy does not count for you, and neither does a copy elsewhere, such as `/usr/bin/biorouter-crew`. Without it, Crew shows "Crew isn’t set up for your account on {server}".

You install it yourself, signed in as yourself, or IT does. A host with an ordinary account cannot install it in your account. That screen says it is installed "usually by your host or IT team", so a host without administrator rights passes your request to IT.

To install it yourself, follow [Install Crew on the server](hosting-a-workspace.md#install-crew-on-the-server) with your own login. Its steps 5 and 6 are the lines under **Install it yourself** on the "Crew isn’t set up" screen. Type the first of those lines yourself, because **Copy** on that field also copies its placeholder path. When the last line prints the version, Crew is installed. If Crew already shows "Crew isn’t set up", choose **Try again** there, and Crew connects.

## Join from the Biorouter app

The invitation is a message with a line that starts `brcrew1:`. It holds no secret. Nobody can join with a copy alone.

1. Open Biorouter. On a Mac or Linux computer, type your approval secret when Biorouter asks ([The approval secret](getting-started.md#the-approval-secret)).
2. Choose **Crew** in the left sidebar, then **Join a workspace**. If you already have a workspace, choose its name at the top of the Crew sidebar, then **Add a workspace** and **Join a workspace…**.
3. Paste the whole message into **Invitation from your host**. The box folds to "Invitation read", and a summary appears. **Edit** reopens the box.
4. Check that the summary names the right workspace and host, for example "Hosted by Alice Chen (@alice) on lab.example.edu". If not, choose **Cancel**.
5. Check **Your username on lab.example.edu**. It is your server login, not your Slack name or email.
6. If Crew asks you to "Choose which AI models may work here", ask your host, then [choose](#which-ai-can-read-the-workspace).
7. Leave **Your agent on lab.example.edu** and **Advanced** alone unless your host or IT said otherwise.
8. Choose **Join chen-lab**. If the server asks for a password, see [Sign in to the server](connections-and-troubleshooting.md#sign-in-to-the-server).

The card with your code appears. Crew finishes the join later without asking again. If a message appears instead, see [When joining does not work](#when-joining-does-not-work). To join from a terminal instead, see [Join a workspace](command-line.md#join-a-workspace).

### Check the invitation fingerprint

Choose **Check this invitation (optional)** to see the workspace fingerprint. Ask your host to read the "Fingerprint" row in their workspace menu. If the two differ, do not join. You never send the fingerprint.

### Which AI can read the workspace

**Private** means "Only private and institution-approved models". **Public** means "Public models allowed for public-safe work". The choice does not change who sees you or what you read. A Private workspace blocks public models whatever you choose. See [Privacy and security](privacy-and-security.md).

To change it:

1. Choose **Advanced**, then **Change which AI can read chen-lab…**.
2. Choose **Private** or **Public**. For Private, type your institution's short ID, such as `ucsf`, in **Institution**. It must match your host's, so ask your host if you do not know it.
3. Choose **Done**. The choice folds into one line, such as "Which AI can read chen-lab: private, ucsf-approved models only".

### Your agent on the server

**Your agent on lab.example.edu** is off by default. To let your Biorouter agent read and write files in a server folder, choose it and type the folder's full path, such as `/home/bob/project`, in **Remote work folder**. **Let my agent run commands in this folder** also lets it run commands there. Change both later in **Connection settings…**. See [Agents and chat access](agents-and-chat-access.md).

### Advanced server connection details

Use **Advanced** only with settings from IT. It opens by itself when the invitation names no server.

| Field | Use |
|---|---|
| **Server login** | A login from IT in place of `bob@lab.example.edu`, or a short name (alias) for the server from your SSH settings file, `~/.ssh/config`. |
| **Port** | A port other than the invitation's. |
| **Identity file** | The full path of an SSH key file, starting with /. |
| **Jump host** | Gateway servers from IT, separated by commas. Each needs [settings in your SSH settings file](getting-started.md#jump-hosts). |
| **Connection name** | Your name for this workspace on this computer. |
| **Enter workspace details manually** | Use only if your host asks. It takes **Your server login** and four values your host copies from the `status` line the start commands printed: **Socket path**, **Workspace ID**, **Host user ID** and **Workspace key**. |

## Send your code to your host

The card reads "Send Alice this code:", then the code and **Copy**.

1. Choose **Copy**. If it reads "Copy failed", type the code.
2. Paste the code into a message to your host, and send it.

Your host can type it with or without dashes, in any case, and can type O as 0. Codes never contain I, L, O or U. Your code stays the same when you reconnect or close Biorouter.

## Wait for your host

Your host may take minutes or hours. Meanwhile the status under the workspace name reads "Not joined yet". Crew checks by itself.

In the workspace menu, **Reconnect** tries again. **Disconnect** stops waiting until you choose **Connect to chen-lab**. Neither changes your code.

You can close Biorouter. When you reopen it, open Crew, and choose **Connect to chen-lab** if Crew shows it. The card shows the same code, or Crew finishes joining if your host let you in meanwhile. See [When you reopen Biorouter](connections-and-troubleshooting.md#when-you-reopen-biorouter).

## When your host lets you in

The card reads "Joining chen-lab…", and the workspace opens.

| You see | Do this |
|---|---|
| Your team's channels | Start writing. |
| "You’re invited to {team}" | Choose **Join {team}**. |
| "You’re in chen-lab" | Ask your host to add you to a team, or choose **Create a team**. |
| "No open channels in {team}" | Choose **Create channel**. |

People see your username until you set a name. If your server account has a full name, a note offers it: "Use “Bob Lee” as your name in chen-lab?" Otherwise choose **Edit profile…** in the You menu at the bottom of the Crew sidebar.

## Add this computer to your existing account

1. Ask your host to invite you with **Add another device for @bob** turned on.
2. On the new computer, join with that invitation, as in [Join from the Biorouter app](#join-from-the-biorouter-app).
3. Send your host the new code.

When your host enters it, the workspace opens on the new computer.

## When joining does not work

### Problems while pasting the invitation

| Message | What to do |
|---|---|
| "This doesn’t look like a Crew invitation…" | Paste the whole message. If that fails, ask your host to copy it again, or update Biorouter. |
| "This feature needs a newer Biorouter background service…" | Follow [Replace an old background service](connections-and-troubleshooting.md#replace-an-old-background-service), then paste again. |
| "You already have chen-lab on this computer." | Choose **Open chen-lab**. |
| "This invitation doesn't match “chen-lab”…" | Do not join. Ask for a new invitation and compare the fingerprint. |
| "This invitation doesn’t name its server…" | Type your server login, such as `bob@lab.example.edu`, in **Server login**. Ask IT if you do not know it. |

### Problems when you choose Join

A message under a field, or in a red note, names the rule a value broke. Correct the value and choose Join again. Other notes:

| Message | What to do |
|---|---|
| "@bob can't be used as an SSH login…" | Ask IT for the login to use, and type it in **Server login** under **Advanced**. |
| "This computer already has “chen-lab” for this workspace…" | Choose **Open chen-lab**. |
| Any other note | Wait a moment and choose Join again. If it repeats, quit and reopen Biorouter, then update it or contact IT. |

### Problems while connecting

[Messages and what to do](connections-and-troubleshooting.md#messages-and-what-to-do) covers every screen.

| Main area | What to do |
|---|---|
| "chen-lab is offline" with "Couldn’t reach lab.example.edu" | Check your network and VPN. Crew keeps trying. |
| "Can’t verify lab.example.edu yet" | [Verify the server](#verify-the-server-on-this-computer), then choose **Try again**. Crew has no button to accept a server. |
| "lab.example.edu’s identity changed" | Do not connect. Send IT the text from **Copy details for IT**, then follow [A changed server identity](connections-and-troubleshooting.md#a-changed-server-identity). |
| "This isn’t the workspace you joined" | Send your host the text from **Copy details**, and wait. |
| "Crew isn’t set up for your account on lab.example.edu" | See [Install Crew in your server account](#install-crew-in-your-server-account). |
| "It didn’t connect" | Choose **Connect to chen-lab** again. If it repeats, ask your host. Crew may have stopped on the server, or the server name may reach a different machine each time. |
| "You already use this server for {other workspace}…" | One computer uses a server for one institution only. Ask your host. |

### Problems while you wait

| Message | What to do |
|---|---|
| "The code Alice entered doesn’t match this computer…" | Send the same code again. |
| "You’re not in chen-lab yet" | Send your host the "message to your host" text. If `bob` is wrong, fix **Your server login** in **Connection settings…** and connect again. |
| "This invitation expired…" | Ask your host to invite you again. The card returns with the same code. |
| "Crew isn’t connected to chen-lab…" | Choose **Reconnect**. |
| "Crew couldn’t check your invitation…" | Wait. If it continues, quit and reopen Biorouter. |

### If joining does not finish

A red note with **Try again** appears under "Joining chen-lab…".

| Message | What to do |
|---|---|
| "Joining didn’t finish." or "Your host sent a new invitation…" | Choose **Try again**. |
| "Your host hasn't let this computer in yet…" | Send your code again. |
| "You're not invited…", "This invitation expired…" or "Your account on the server changed…" | Ask for a new invitation. |
| "This workspace's server doesn't support joining by invitation yet…" | [Join with a token](#join-with-a-token-on-an-older-server). |
| "Another member of this workspace already has your username…", "This computer's key is already in this workspace." or "The workspace didn't let this computer join." | Tell your host. |

"You’re no longer in chen-lab" after you joined means your host removed this computer or your account. Ask your host.

## Less common situations

### Join with a token on an older server

An older Crew server cannot take a code, so Crew shows "Join with an invitation token".

1. Copy the join request and send it to your host. Its device key is public.
2. Paste the token your host sends into **Token from an older invitation**. It works once, within an hour.
3. Choose **Join workspace**. Crew joins, and the workspace opens.

If your host asks for a join request, or sends a token, while your code card shows, choose **Having trouble joining?**, then **Show the join request** or **Alice sent me a token instead**. If a token is refused, ask for a new one.

## Related documentation

- [Getting started](getting-started.md): the approval secret and jump hosts.
- [Hosting a workspace](hosting-a-workspace.md): your host's side.
- [Teams, channels and people](teams-channels-and-people.md): your name and teams.
- [Privacy and security](privacy-and-security.md): Private, Public and fingerprints.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connection problems.
- [Command line](command-line.md): every `biorouter crew` command.
- [Administration](administration.md): server setup.
