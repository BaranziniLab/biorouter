# Joining a workspace

> **What this is.** How to join your lab's Crew workspace, open it on another computer, add a computer again, and what to do when a step stops you.
> **Status:** Current. Checked against the Crew code on 2026-11-06, after the account ownership change ([design](design/account-ownership.md)).
> **Audience:** Lab members who received a Crew invitation, or who use a workspace from more than one computer. You need no SSH experience.

To join, verify the lab server once, paste the invitation from your host (the person who created the workspace), choose what your agent may do on the server, and choose Join. If Crew's server program is not in your server account yet, Biorouter installs it there when you choose Join. On a server with a current Crew, the workspace opens at once. On a server with an older Crew, Crew shows a code: send it to your host, and the workspace opens when your host, or in an open workspace any member, enters it.

Examples use the host Alice Chen (@alice), the workspace `chen-lab` on `lab.example.edu`, and your username `bob`. Braces, such as {team}, mark a name Crew fills in.

## You are your server account

Crew knows you by your account on the lab server. Every computer that signs in to `lab.example.edu` as `bob` is @bob, in every workspace on that server. The account that hosts a workspace is its host, from any computer that signs in as it.

This is how the server's Crew works once it is current. It means:

- You need no code to join. The invitation names your account, and signing in as that account is the proof.
- A new computer of yours opens your workspaces without an invitation. See [Open a workspace on a server](#open-a-workspace-on-a-server).
- A computer that lost its key gets a new one with **Add this computer again**. See [Add this computer again](#add-this-computer-again).
- Anyone who can sign in to the server as your account can use Crew as you. Keep your SSH keys and password to yourself, and see [Your computers](privacy-and-security.md#your-computers) to check which computers use Crew as you.

A server whose Crew is older still joins by code, and the steps for it are marked "older Crew" below. Your host updates the server with `biorouter crew setup` ([Keep Crew current on the server](hosting-a-workspace.md#keep-crew-current-on-the-server)).

## Before you start

| You need | If it is missing |
|---|---|
| The Biorouter desktop app. | See [Installation and setup](../getting-started/installation.md). |
| An account on the lab server, reached with SSH (Secure Shell, how your computer signs in to a server). | Ask your IT team. |
| Your exact server username, sent to your host before they invite you. It is the name before the @ in `ssh bob@lab.example.edu`. `bob` does not match `crew_bob`. | Ask IT for it. |
| The invitation. It lasts 24 hours. | Ask your host to invite you again. |
| The server verified on this computer. | Follow [Verify the server on this computer](#verify-the-server-on-this-computer). |
| `biorouter-crew` in your own account on the server. | Biorouter installs it when you choose Join. See [Install Crew in your server account](#install-crew-in-your-server-account). |

### Verify the server on this computer

Crew connects only to a server whose key is in your known hosts file, `~/.ssh/known_hosts`, and never adds one for you. Do this once per server on each computer.

1. Get the server fingerprint. It starts with `SHA256:`. Your host may have sent it with the invitation. If not, ask IT. If IT gave you a jump host (a server you pass through first), ask IT for its fingerprint too. A server fingerprint is neither the workspace fingerprint nor your code.
2. Open a terminal. On a Mac, press Command and Space, type Terminal, and press Return. Crew's "Can’t verify" screen also offers **How do I verify it?**, then **Open a terminal here**.
3. If IT gave you a jump host, verify it on its own first. Type `ssh -o StrictHostKeyChecking=ask gateway.ucsf.edu` with the jump host's name, and follow steps 5 to 8 for it. IT's settings for a jump host make SSH check it strictly, so without this step SSH never asks about it and stops with "No ED25519 host key is known for gateway.ucsf.edu and you have requested strict checking." If IT added the jump host's key for you, skip this step.
4. Type `ssh bob@lab.example.edu` with your username and server, and press Enter.
   - For a port other than 22, add `-p` and the port: `ssh -p 2222 bob@lab.example.edu`.
   - For a jump host, add `-J` and its name: `ssh -J gateway.ucsf.edu bob@lab.example.edu`.
   - If you do not know the server or its port, paste the invitation in the Join dialog (steps 1 to 3 of [Join from the Biorouter app](#join-from-the-biorouter-app)). The server is the name after "on" in the summary. Choose **Advanced**: the grey number in **Port** is the port. Choose **Cancel**. Nothing is saved.
5. Read the answer:
   - "Are you sure you want to continue connecting", after a `SHA256:` value: go to step 6.
   - A password prompt, or the server's prompt, such as `bob@lab:~$`: the server is already verified. Press Ctrl+C at a password prompt, or type `exit` at the server's prompt.
   - "No … host key is known for … and you have requested strict checking": that host is a jump host you have not verified. Do step 3 for it.
   - "Could not resolve hostname": check the spelling of the server name, or ask IT.
   - "timed out" or "Connection refused": connect to your VPN (the app that connects you to your institution's network), check the port, and try again.
   - "Permission denied": ask IT which login or key to use.
   - "REMOTE HOST IDENTIFICATION HAS CHANGED": stop, and send the whole message to IT. When IT confirms the change, remove the old key (step 2 of [A changed server identity](connections-and-troubleshooting.md#a-changed-server-identity)), and start again at step 4.
6. Compare the `SHA256:` value with the one you got for that host, character by character. If a value differs, type `no` and tell IT.
7. If it matches, type `yes` and press Enter. SSH prints "Permanently added … to the list of known hosts." If SSH then asks about the next host, repeat step 6.
8. Sign in with your password or code. The terminal does not show a password. Then type `exit`. For a jump host checked in step 3, type `exit` and go on to step 4.
9. In Crew, choose **Try again**, or join as below.

Typing `yes` is the step Crew's help calls adding "the full key". Crew may then ask for your password again.

If Crew's "Can’t verify" screen names your jump host rather than the server, the jump host is the one to verify: do step 3 for it.

### Install Crew in your server account

Crew needs its server program at exactly `~/.local/bin/biorouter-crew` in your own account on the lab server. Your host's copy does not count for you, and neither does a copy elsewhere, such as `/usr/bin/biorouter-crew`.

You usually do nothing. When you choose Join, Biorouter signs in to the server as you, with your SSH settings, and installs it there if it is missing, from the copy built into Biorouter. It needs no administrator rights. If the server asks for a password or a code, the Sign in window opens, and one sign-in covers the install and the connection. The Join button reads "Setting up Crew on {server}…" while it works.

If Crew later shows "Crew isn’t set up for your account on {server}", for example after you removed the program:

1. Choose **Set up Crew for my account**. Biorouter installs it and connects. This changes nothing about what your agent may do on the server.
2. If that fails, the screen says why ([Setting Crew up](connections-and-troubleshooting.md#setting-crew-up-in-your-server-account)). **Ask your host instead** gives a message to send, and **Install it yourself** gives the commands to install it by hand, which [Install Crew yourself](hosting-a-workspace.md#install-crew-yourself) explains.
3. When someone else installed it for you, choose **Try again**, and Crew connects.

## Join from the Biorouter app

The invitation is a message with a line that starts `brcrew1:`. It holds no secret. Nobody can join with a copy alone.

1. Open Biorouter.
2. Choose **Crew** in the left sidebar, then **Join a workspace**. If you already have a workspace, choose its name at the top of the Crew sidebar, then **Add a workspace** and **Join a workspace…**.
3. Paste the whole message into **Invitation from your host**. The box folds to "Invitation read", and a summary appears. **Edit** reopens the box.
4. Check that the summary names the right workspace and host, for example "Hosted by Alice Chen (@alice) on lab.example.edu". If not, choose **Cancel**.
5. Check **Your username on lab.example.edu**. It is your server login, not your Slack name or email. If the invitation was made for another account than the one this computer signs in as, a note says so, such as "This invitation is for @crew_bob, but this computer signs in to lab.example.edu as bob. Ask your host for your own invitation." Ask your host for an invitation to your own username.
6. If Crew asks you to "Choose which AI models may work here", ask your host, then [choose](#which-ai-can-read-the-workspace).
7. Check **Your agent on lab.example.edu**, which says what your agent may do on the server. It starts at **As me on the server**. See [Your agent on the server](#your-agent-on-the-server).
8. Leave **Advanced** alone unless your host or IT said otherwise.
9. Choose **Join chen-lab**. Under it, Crew says "If Crew isn’t set up for your account on lab.example.edu yet, Biorouter installs it there for you." If the server asks for a password, see [Sign in to the server](connections-and-troubleshooting.md#sign-in-to-the-server).

On a server with a current Crew, Crew joins as your account, and the workspace opens ([When you are in](#when-you-are-in)). On an older Crew, the card with your code appears ([Send your code to your host](#send-your-code-to-your-host-older-crew)), and Crew finishes the join later without asking again. If a message appears instead, see [When joining does not work](#when-joining-does-not-work). To join from a terminal instead, see [Join a workspace](command-line.md#join-a-workspace). To join from a terminal on the server that runs the workspace, see [Join from the workspace's own server](command-line.md#join-from-the-workspaces-own-server).

Crew joins by itself only when you started the join, with Join or **Open**. When this computer has a workspace saved that it has not joined yet, and you did not just start it, the card reads "You can join chen-lab" and "You sign in to lab.example.edu as bob, so this computer can join without a code." Choose **Join chen-lab**. The card then reads "Joining chen-lab…".

### Check the invitation fingerprint

Choose **Check this invitation (optional)** to see the workspace fingerprint. Ask your host to read the "Fingerprint" row in their workspace menu. If the two differ, do not join. You never send the fingerprint.

### Which AI can read the workspace

**Private** means "Only private and institution-approved models". **Public** means "Public models allowed for public-safe work". The choice does not change who sees you or what you read. A Private workspace blocks public models whatever you choose. See [Privacy and security](privacy-and-security.md).

To change it:

1. Choose **Advanced**, then **Change which AI can read chen-lab…**.
2. Choose **Private** or **Public**. For Private, type your institution's short ID, such as `ucsf`, in **Institution**. It must match your host's, so ask your host if you do not know it.
3. Choose **Done**. The choice folds into one line, such as "Which AI can read chen-lab: private, UCSF-approved models only". The line uses the name your model provider publishes for the institution, such as "UCSF", when one of your models names it, and otherwise the short ID you typed. Always type the short ID, such as `ucsf`, in **Institution**.

### Your agent on the server

**Your agent on lab.example.edu** answers "What can your agent do on lab.example.edu?" with three levels. You choose it for your own account, and nobody else can change it. Public models never get any of them.

| Level | What your agent may do |
|---|---|
| **Off** | "Your agent doesn’t use this server." |
| **In a folder** | "It reads and writes one folder, and can run commands there in a sandbox." Type the folder's full path, such as `/home/bob/project`, in **Remote work folder**, and turn on **Let my agent run commands in this folder** to let it run commands there. A server whose Linux kernel cannot confine commands to a folder (older than 6.2) gives a folder files only, and commands there need **As me on the server**. |
| **As me on the server** | Full access: "It runs commands as bob, with your files, programs and network." A work folder is optional: leave it empty, and Biorouter creates one in `~/crew-work`, named for the workspace. |

**As me on the server** comes with one warning, which you read before you choose Join: "Your agent will run commands as bob on lab.example.edu: it can read and change anything your account can, use the network, and start programs that keep running after access ends. Other people’s messages and files in channels it reads can try to steer it. Use a private model you trust." See [What your agent may do on the server](agents-and-chat-access.md#what-your-agent-may-do-on-the-server).

Joining gives your connection exactly the level you chose. **Off** stays off once the connection has connected: setting Crew up never turns access on by itself. One case is different: a connection saved but never connected from this computer (the desktop's setup failed, for example) that you then join with `biorouter crew join` gets the same level `biorouter crew setup` gives a new connection, **As me on the server**, and the terminal says so before anything runs. Change the level later in **Connection settings…**.

### Advanced server connection details

Use **Advanced** only with settings from IT. It opens by itself when the invitation names no server.

When you leave **Server login**, **Port** and **Jump hosts** empty, Crew saves the login `bob@lab.example.edu`. If your SSH settings file, `~/.ssh/config`, has a short name (alias) for that server that signs in as the same account on the same port, Crew saves that name instead, so the key, port and jump host you set for it there apply. The summary then names the server by that name. A short name that signs in as another account, or on another port, is never used, and neither is one Crew could not connect through, such as one with a `ProxyCommand` or a `RemoteCommand`. When you join on the workspace's own server, the login is `bob@localhost`.

| Field | Use |
|---|---|
| **Server login** | A login from IT in place of `bob@lab.example.edu`, or a short name (alias) for the server from your SSH settings file, `~/.ssh/config`. |
| **Port** | A port other than the invitation's. |
| **Identity file** | The full path of an SSH key file, starting with /. |
| **Jump hosts** | Gateway servers from IT, separated by commas. Each needs [settings in your SSH settings file](getting-started.md#jump-hosts). |
| **Connection name** | Your name for this workspace on this computer. |
| **Enter workspace details manually** | Use only if your host asks. It takes **Your server login** and four values your host copies from the `status` line the start commands printed: **Socket path**, **Workspace ID**, **Host user ID** and **Workspace key**. |

## Send your code to your host (older Crew)

This section and the next apply only on a server with an older Crew. A current Crew shows no code.

The card reads "Send Alice this code:", then the code and **Copy**.

1. Choose **Copy**. If it reads "Copy failed", type the code.
2. Paste the code into a message to your host, and send it.

Your host can type it with or without dashes, in any case, and can type O as 0. Codes never contain I, L, O or U. Your code stays the same when you reconnect or close Biorouter.

## Wait for your host (older Crew)

Your host may take minutes or hours. In an open workspace, any member can let you in with your code. Meanwhile the status under the workspace name reads "Not joined yet". Crew checks by itself.

In the workspace menu, **Reconnect** tries again. **Disconnect** stops waiting until you choose **Connect to chen-lab**. Neither changes your code.

You can close Biorouter. When you reopen it, open Crew, and choose **Connect to chen-lab** if Crew shows it. The card shows the same code, or Crew finishes joining if your host let you in meanwhile. See [When you reopen Biorouter](connections-and-troubleshooting.md#when-you-reopen-biorouter).

## When you are in

The card reads "Joining chen-lab…", and the workspace opens. In an open workspace you are in every team and every channel that is not archived, so your channels are there at once. In a managed workspace, your host adds you to a team.

| You see | Do this |
|---|---|
| Your team's channels | Start writing. |
| "You’re invited to {team}" | Choose **Join {team}**. |
| "You’re in chen-lab" | Ask your host to add you to a team, or choose **Create a team**. |
| "No open channels in {team}" | Choose **Create channel**. |

People see your username until you set a name. If your server account has a full name, a note offers it: "Use “Bob Lee” as your name in chen-lab?" Otherwise choose **Edit profile…** in the You menu at the bottom of the Crew sidebar.

## Open a workspace on a server

Use this on a new computer, or after you removed a workspace from this one, to open a workspace your server account already belongs to, is invited to, or hosts. You need no invitation and no code. It needs a current Crew on the server.

1. [Verify the server on this computer](#verify-the-server-on-this-computer), if you have not on this computer.
2. Open Crew and choose **Open a workspace on a server**. If you already have a workspace, choose its name at the top of the Crew sidebar, then **Add a workspace** and **Open a workspace on a server…**.
3. Type your server login in **Your server login**, such as `bob@lab.example.edu`. Use **Advanced** only for a port, key file or jump host from IT.
4. Choose **Look for workspaces**. If the server asks for a password or a code, the Sign in window opens. Type it there.
5. Under "Workspaces on lab.example.edu", Crew lists the workspaces there for your account. Each names its host, such as "chen-lab, hosted by @alice", or "chen-lab, hosted by you", then your role ("You host it", "You’re a member", "You’re invited" or "You were removed"), its state, such as "Running" or "Stopped", its Crew version, and "Open on this computer" or "Saved on this computer" when this computer already has it. Choose the button beside the one you want.

| Button | When it shows | What it does |
|---|---|---|
| **Open** | You are a member of the workspace, invited to it, or its host, and it is running. | Saves the workspace on this computer, connects and joins as your account. The workspace opens. |
| **Start** | You host the workspace, and it is stopped, for example after the server restarted. | Starts it on the server, and says "Started chen-lab on Crew {version}.". Then choose **Open**. |
| **Update Crew** | You host the workspace, it is running, and it runs an older Crew than this Biorouter has. | Updates Crew in your account and restarts the workspace on it, then says "Restarted chen-lab on Crew {version}.". When other people are connected, Crew asks first: "2 people are connected. Their running agent commands will stop." Choose **Update and restart**, or **Not now**. |

A workspace you were removed from has no button, and reads "You were removed from chen-lab. Ask a member to invite you again." When the list is empty, Crew says "There are no workspaces for you on lab.example.edu. To join someone else’s, ask them to invite you."

Crew trusts a workspace found this way because the server vouches for it: it runs in a folder only its host's account controls, as a program running as that account. Check that the host Crew names is the one you expect. When two hosts on the server have a workspace with the same name, Crew says "Two people host a workspace named chen-lab here. Check the host before you open one.", and names each host in the list.

From a terminal, `biorouter crew open chen-lab --server bob@lab.example.edu` does the same ([Open a workspace from a terminal](command-line.md#open-a-workspace-from-a-terminal)).

## Add this computer again

A computer can lose its key for a workspace, for example after you reinstalled the computer or cleared its keychain, or after you removed it in **Keys and security…** ([Your computers](privacy-and-security.md#your-computers)). The connection bar then says one of these:

- "This computer no longer has its key for {workspace}. Choose Add this computer again to make a new one."
- "{workspace} no longer recognizes this computer. Choose Add this computer again while you are still a member."
- "You removed this computer from {workspace}. Choose Add this computer again to use it here."

To fix it:

1. Choose **Add this computer again** in the connection bar. It reads "Adding this computer…" while it works.
2. If the server asks for a password or a code, sign in.

Crew makes a new key for this computer and adds it to the workspace as your account. The bar then says "This computer is back in {workspace}. Chats that had Crew access through it need access again." The saved connection stays, with its settings. Grant a chat its access again with `/crew`. **Reconnect** is a different action: it only dials the server again with the key this computer has.

A workspace keeps up to 32 computers for each person. When a new one would be the 33rd, the workspace removes your oldest computer, and the bar adds "To make room, the workspace removed your oldest device, {fingerprint}."

From a terminal, run `biorouter crew rejoin` ([Add this computer again from a terminal](command-line.md#add-this-computer-again-from-a-terminal)). It needs a current Crew on the server.

## Add this computer to your existing account

On a server with a current Crew, use [Open a workspace on a server](#open-a-workspace-on-a-server). Your new computer joins as your account, and you keep your teams and channels. Your host cannot invite you again while you are a member: their Invite dialog says "@bob is already in chen-lab. A new computer of theirs gets in when they open chen-lab on it, signed in to the server as @bob."

On a server with an older Crew:

1. Ask your host to invite you with **Add another device for @bob** turned on.
2. On the new computer, join with that invitation, as in [Join from the Biorouter app](#join-from-the-biorouter-app).
3. Send your host the new code.

When your host enters it, the workspace opens on the new computer.

## When joining does not work

### Problems while pasting the invitation

| Message | What to do |
|---|---|
| "This doesn’t look like a Crew invitation…" | Paste the whole message. If that fails, ask your host to copy it again. |
| "This invitation is incomplete or was changed. Paste the whole message again, or ask your host to send it again." | The copy you pasted was cut short, or something changed a character in it. Paste the whole message again. If it happens again, ask your host to send it again. An invitation that email or chat broke across lines is read whole, so line breaks are not the problem. |
| "This invitation needs a newer Biorouter…" | Update Biorouter, then paste it again. |
| "This feature needs a newer Biorouter background service…" | Follow [Replace an old background service](connections-and-troubleshooting.md#replace-an-old-background-service), then paste again. |
| "You already have chen-lab on this computer." | Choose **Open chen-lab**. To sign in with another account, choose **Connection settings…** and change **Your server login**. Pasting again changes nothing. |
| "This computer has a saved connection to chen-lab that never joined. Joining replaces it." | The saved connection came from an invitation that could not work here, such as someone else's. Choose **Replace the saved connection** to join with this invitation. |
| "This invitation doesn't match “chen-lab”…" | Do not join. Ask for a new invitation and compare the fingerprint. |
| "This invitation doesn’t name its server…" | Type your server login, such as `bob@lab.example.edu`, in **Server login**. Ask IT if you do not know it. |

### Problems when you choose Join

A message under a field, or in a red note, names the rule a value broke. Correct the value and choose Join again. Other notes:

| Message | What to do |
|---|---|
| "@bob can't be used as an SSH login…" | Ask IT for the login to use, and type it in **Server login** under **Advanced**. |
| "This computer already has “chen-lab” for this workspace. Change it in its connection settings instead." | This computer joined chen-lab before. Choose **Open chen-lab**. To sign in with another account, choose **Connection settings…** in the workspace menu and change **Your server login**. |
| "This computer already has “chen-lab” for this workspace, signing in as another account, and it has never connected. Replace it with this invitation, or change it in its connection settings." | The saved connection came from an invitation that could not work here, such as someone else's. Close the dialog, then join again with your own invitation. After you paste it, the dialog says the saved connection will be replaced, and the button reads **Replace the saved connection**. Choose it. From a terminal, run `connections join-invitation` again with `--replace` ([Join a workspace](command-line.md#join-a-workspace)). |
| Any other note | Wait a moment and choose Join again. If it repeats, quit and reopen Biorouter, then update it or contact IT. |

### Problems while connecting

[Messages and what to do](connections-and-troubleshooting.md#messages-and-what-to-do) covers every screen.

| Main area | What to do |
|---|---|
| "chen-lab is offline" with "Couldn’t reach lab.example.edu" | Check your network and VPN. Crew keeps trying. |
| "Can’t verify lab.example.edu yet" | [Verify the server](#verify-the-server-on-this-computer), then choose **Try again**. Crew has no button to accept a server. |
| "lab.example.edu’s identity changed" | Do not connect. Send IT the text from **Copy details for IT**, then follow [A changed server identity](connections-and-troubleshooting.md#a-changed-server-identity). |
| "This isn’t the workspace you joined" | Send your host the text from **Copy details**, and wait. |
| "Crew isn’t set up for your account on lab.example.edu" | Choose **Set up Crew for my account**. See [Install Crew in your server account](#install-crew-in-your-server-account). |
| "Can’t sign in to lab.example.edu" with "lab.example.edu refused this computer’s SSH key for bob." | The server takes no password here, so signing in cannot help. Check **Your server login** in **Connection settings…**, and ask IT which login and key to use. |
| "Crew isn’t running on lab.example.edu" with "The workspace server isn’t running. Once Alice Chen starts Crew, this computer connects by itself within a few minutes, or you can connect now." | The workspace stopped on the server, for example after it restarted. Ask your host to start Crew if they have not. Crew connects by itself within about a minute of the start, or choose **Connect to chen-lab** to try at once. |
| "It didn’t connect" | Choose **Connect to chen-lab** again. If it repeats, ask your host. The server name may reach a different machine each time. |
| "You already use this server for {other workspace}…" | One computer uses a server for one institution only. Ask your host. |
| "This computer no longer has its key for chen-lab…" or "chen-lab no longer recognizes this computer…" | Choose **Add this computer again**. See [Add this computer again](#add-this-computer-again). |

### Problems while you wait

| Message | What to do |
|---|---|
| "You’re no longer in chen-lab", with "You were removed from chen-lab. Ask a member to invite you again." | Your account was removed from the workspace. Ask a member for a new invitation, then join with it. |
| "chen-lab's server runs an older Crew. Ask @alice to update it." | Opening a workspace by account and **Add this computer again** need a current Crew on the server. Ask your host to update it. A host reads "chen-lab's server runs an older Crew. Update it with biorouter crew setup --server alice@lab.example.edu." and runs that command ([Keep Crew current on the server](hosting-a-workspace.md#keep-crew-current-on-the-server)). |
| "The code Alice entered doesn’t match this computer…" | Send the same code again. |
| "You’re not in chen-lab yet" | Send your host the "message to your host" text, which on a server with a current Crew reads "Hi Alice, please invite @bob to chen-lab in Crew." If `bob` is wrong, fix **Your server login** in **Connection settings…** and connect again. On a server with an older Crew, the card also says "If you were in chen-lab before, ask Alice Chen (@alice) to use Add another device for @bob." Do that if you were a member before, for example if you removed chen-lab from this computer. |
| "This invitation expired…" | Ask your host to invite you again. The card returns with the same code. |
| "Crew isn’t connected to chen-lab…" | Choose **Reconnect**. |
| "Crew couldn’t check your invitation…" | Wait. If it continues, quit and reopen Biorouter. |

### If joining does not finish

A red note with **Try again** appears under "Joining chen-lab…".

| Message | What to do |
|---|---|
| "Joining didn’t finish." or "Your host sent a new invitation…" | Choose **Try again**. |
| "Your host hasn't let this computer in yet…" | Send your code again. |
| "You're not invited…", "This invitation expired…" or "Your account on the server changed…" | Ask for a new invitation. An invitation from a member also stops working once that member is removed or the workspace stops being open, so ask the host for one. |
| "This workspace's server doesn't support joining by invitation yet…" | [Join with a token](#join-with-a-token-on-an-older-server). |
| "Another member of this workspace already has your username…", "This computer's key is already in this workspace." or "The workspace didn't let this computer join." | Tell your host. |

"You’re no longer in chen-lab" after you joined means your host removed your account. Ask your host. On a server with a current Crew, a workspace that only stopped recognizing this computer, while your account is still a member, offers **Add this computer again** instead ([Add this computer again](#add-this-computer-again)).

## Less common situations

### Join with a token on an older server

A Crew server from before codes cannot take a code, so Crew shows "Join with an invitation token".

1. Copy the join request and send it to your host. Its device key is public.
2. Paste the token your host sends into **Token from an older invitation**. It works once, within an hour.
3. Choose **Join workspace**. Crew joins, and the workspace opens.

If your host asks for a join request, or sends a token, while your code card shows, choose **Having trouble joining?**, then **Show the join request** or **Alice sent me a token instead**. If a token is refused, ask for a new one.

## Related documentation

- [Getting started](getting-started.md): the background service and jump hosts.
- [Hosting a workspace](hosting-a-workspace.md): your host's side, and keeping Crew current on the server.
- [Account ownership](design/account-ownership.md): the design behind joining as your server account, for developers.
- [Teams, channels and people](teams-channels-and-people.md): your name and teams.
- [Privacy and security](privacy-and-security.md): Private, Public and fingerprints.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connection problems.
- [Command line](command-line.md): every `biorouter crew` command.
- [Administration](administration.md): server setup.
