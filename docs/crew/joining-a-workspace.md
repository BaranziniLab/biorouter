# Joining a workspace

> **What this is.** Step by step instructions for joining your lab's Crew workspace from the Biorouter desktop app, with every message that can stop you and what to do about it.
> **Status:** Current.
> **Audience:** Lab members who received a Crew invitation from their host. You need no experience with SSH (Secure Shell, the way your computer signs in to the lab server). The page gives every terminal command you type.

Crew is the part of Biorouter where your lab chats, shares files and runs agents together. Your lab's workspace runs on a Linux server, and the person who set it up is the host. To join, you paste the invitation your host sent you, choose Join, send your host a short code, and wait for your host to let you in. Crew then opens the workspace by itself.

The examples on this page use a host named Alice Chen (@alice), a workspace named `chen-lab` on the server `lab.example.edu`, the institution UCSF, and your username `bob`. Your screen shows your own names. In a few labels, text in braces, such as {team}, stands for a name Crew fills in.

## Words used on this page

| Word | Meaning |
|---|---|
| Host | The person who created the workspace. The host invites people and lets them in. |
| Workspace | Your lab's shared space in Crew: its teams, channels, messages and files. |
| Server | The lab computer where the workspace runs. You sign in to it with your own account. |
| SSH (Secure Shell) | The secure way your computer signs in to the lab server over a network. Crew uses it to reach the workspace. |
| Approval secret | A secret of your own that Biorouter's background service asks for before it accepts your actions. On a Mac or Linux computer, Biorouter asks for it each time it opens. |
| Invitation | The message your host sends you. It tells Crew which workspace to join and which server it is on. |
| Your code | A 16 character code Crew shows after you choose Join. You send it to your host. |
| Workspace fingerprint | A short value that identifies the workspace, such as "6682 327B A040 C709". You can use it to check an invitation. You never send it. |
| Server fingerprint | A value that identifies the server computer. It starts with `SHA256:`. Your IT team gives it to you. You use it once, to verify the server on this computer. |

The workspace fingerprint and the server fingerprint are different values. Neither one is your code.

## Before you start

You need all seven items in this table. The last column says what happens when one is missing.

| You need | Why | If it is missing |
|---|---|---|
| The Biorouter desktop app | Crew is part of it. | Install Biorouter first. See [Installation and setup](../getting-started/installation.md). |
| An approval secret you choose the first time Biorouter opens (on a Mac or Linux computer) | Biorouter's background service accepts your Crew actions only from someone who knows this secret. | Biorouter does not open until you type one. Choose a secret of 32 to 4096 characters with no spaces, and keep it in your password manager. See [The approval secret](getting-started.md#the-approval-secret). |
| An account on the lab server, and a way to sign in to it: an SSH key, a password, or a verification code | Crew reaches the workspace through your own login on the server. | Ask your IT team for an account. |
| Your exact username on the server, sent to your host before they invite you | Your host invites you by your login name on the server. It is the name before the @ when you sign in with SSH, for example `bob` in `ssh bob@lab.example.edu`. A name that is close is not enough: `bob` fails when the account is `crew_bob`. | Your host's Crew refuses the name, and no invitation reaches you. Ask your IT team for your exact username, then send it to your host. |
| The invitation from your host | It names the workspace and its server. | Ask your host. An invitation lasts 24 hours from the moment your host invites you. |
| Crew installed for your account on the server | Crew runs a small program, `biorouter-crew`, under your account. Your host or IT team installs it once for each account. | You see "Crew isn’t set up for your account on lab.example.edu". See [Crew is not set up for your account](#crew-is-not-set-up-for-your-account). |
| The server verified on this computer | Crew connects only to servers whose full key is already saved on your computer. Crew never saves a new server key by itself. | You see "Can’t verify lab.example.edu yet". Follow [Verify the server on this computer](#verify-the-server-on-this-computer). |

Do the last item before you choose Join. To check whether it is done:

1. Open a terminal on this computer. On a Mac, press Command and Space, type Terminal, and press Return. You can also open **Applications**, then **Utilities**, then **Terminal**.
2. Type `ssh bob@lab.example.edu`, with your own username and server, and press Return. If you do not know the server's address, find it first with the steps under step 3 of [Verify the server on this computer](#verify-the-server-on-this-computer).
3. Read the answer:
   - If the terminal asks "Are you sure you want to continue connecting", the server is not verified yet. Type `no`, press Return, and follow [Verify the server on this computer](#verify-the-server-on-this-computer).
   - If the terminal asks for your password, or shows the server's prompt, the server is verified. Press Control and C to stop at the password prompt, or type `exit` at the server's prompt.
   - If the terminal shows "REMOTE HOST IDENTIFICATION HAS CHANGED", do not continue. See [The server's identity changed](#the-servers-identity-changed).

### Verify the server on this computer

You do this once for each server, on each computer. It saves the server's full key in your known hosts file, usually `~/.ssh/known_hosts`. Crew reads that file every time it connects.

1. Get the server fingerprint from your IT team or your institution's directory. It starts with `SHA256:`. It is not the workspace fingerprint in the Join dialog.
2. Open a terminal. On a Mac, press Command and Space, type Terminal, and press Return. If Crew already shows "Can’t verify lab.example.edu yet", you can instead choose **How do I verify it?**, then **Open a terminal here**. That opens a terminal inside Biorouter, on this computer.
3. Type `ssh bob@lab.example.edu`, with your own username and server, and press Return.
   - If you do not know the server's address, read it from the invitation first:
     1. In Biorouter, choose **Crew** in the left sidebar, then **Join a workspace**. If you already have a workspace, open the dialog as described in [Open the Join dialog](#open-the-join-dialog).
     2. Paste the invitation into **Invitation from your host**.
     3. Read the address after "on" in the summary, for example "Hosted by Alice Chen (@alice) on lab.example.edu".
     4. Choose **Cancel**. Crew saves nothing. Do not choose **Join chen-lab** yet.
     5. Type the `ssh` command with that address, and continue with step 4 below.
   - If your server uses a port other than 22, type `ssh -p 2222 bob@lab.example.edu`, with your port in place of 2222.
   - If your IT team gave you a jump host, type `ssh -J gateway.ucsf.edu bob@lab.example.edu`, with your jump host in place of `gateway.ucsf.edu`. SSH asks you to confirm the jump host first. Check its fingerprint the same way.
4. The terminal shows lines like these. The exact lines depend on your version of SSH, and the key type can also read ECDSA or RSA.

   ```text
   The authenticity of host 'lab.example.edu (192.0.2.10)' can't be established.
   ED25519 key fingerprint is SHA256:<43 letters, numbers and symbols>.
   Are you sure you want to continue connecting (yes/no/[fingerprint])?
   ```

5. Compare the `SHA256:` value in the terminal with the one from your IT team, character by character.
6. If they match, type `yes` and press Return. SSH prints a line such as "Permanently added 'lab.example.edu' (ED25519) to the list of known hosts." That line means the full key is now saved.
   - Instead of `yes`, you can paste the `SHA256:` value from your IT team and press Return. SSH saves the key only if the two values match. If they differ, SSH asks again.
7. If they do not match, type `no` and press Return. Do not connect to this server. Tell your IT team.
8. If SSH asks for your password, with a prompt such as `bob@lab.example.edu's password:`, type your password and press Return. The terminal does not show the characters as you type. If it asks for a verification code, type the code. If you sign in with an SSH key, SSH asks for nothing.
9. When the server's prompt appears, type `exit` and press Return.
10. If Crew shows "Can’t verify lab.example.edu yet", choose **Try again** there. If you have not joined yet, open the Join dialog again, paste the invitation, and [join from the Biorouter app](#join-from-the-biorouter-app) as usual.

Crew's own help reads "add the full key to your known-hosts file. A fingerprint alone isn’t enough." Typing `yes` in step 6 is that step. Reading the fingerprint, or pasting it anywhere else, saves nothing.

After you choose **Try again**, or **Join chen-lab** in the Join dialog, Crew makes its own connection to the server. It does not reuse the terminal session you closed. If the server asks for a password or a code, the "Sign in to lab.example.edu" window opens, and you sign in once more there. See [Sign in to the server](#sign-in-to-the-server). Crew then shows the card with your code. See [Send your code to your host](#send-your-code-to-your-host).

## The invitation your host sends

Your host sends the invitation by Slack, email or any other way you usually talk. It looks like this:

```text
Join chen-lab on Crew.
In Biorouter, open Crew, choose Join a workspace, and paste this whole message.
brcrew1:<a long line of letters and numbers>
```

- Paste the whole message. Other text around it, such as a greeting or a signature, does no harm. Crew reads the line that starts with `brcrew1:`.
- The invitation holds no password or other secret. A person who gets a copy cannot join with it. That person would also need the server account your host invited, and your host's approval of their code.
- The invitation expires 24 hours after your host invites you. If it expires, ask your host to invite you again. A new invitation replaces the old one.

## Join from the Biorouter app

### Open the Join dialog

1. Open Biorouter. On a Mac or Linux computer, a window asks for your approval secret before the app appears. See [The approval secret](getting-started.md#the-approval-secret).
   - The first time, the window is titled "Set approval secret for shared BioRouter daemon". Type a secret you choose. A second window, "Confirm shared daemon approval secret", asks for the same secret again.
   - When Biorouter's background service is already running, as it is when you reopen Biorouter without restarting your computer, the window is titled "Connect to existing BioRouter daemon". Type the secret you set before.
2. Choose **Crew** in the left sidebar.
3. Open the dialog:
   - If this computer has no Crew workspace yet, the page reads "Work together in Crew". Choose **Join a workspace**.
   - If you already have a workspace, choose the workspace name at the top of the Crew sidebar. In the menu that opens, choose **Add a workspace**, then **Join a workspace…**.

The "Join a workspace" dialog opens. The cursor is already in the box labeled **Invitation from your host**.

### Paste the invitation

1. Copy the whole message your host sent you.
2. Paste it into **Invitation from your host**.
3. Wait a moment. Under the box, "Reading the invitation…" appears while Crew reads it. Nothing is saved at this point, and the Join button stays unavailable until Crew has finished reading.
4. When Crew has read the invitation, the box folds into one line: a check mark, "Invitation read", and an **Edit** link. A summary of the invitation appears below it.

To use a different invitation, choose **Edit** and paste the new one. Typing in the box never folds it. Only a paste does.

If a message appears under the box instead of a summary, see [Problems while pasting the invitation](#problems-while-pasting-the-invitation).

### Check the summary

The summary shows, from top to bottom:

1. The workspace name, for example `chen-lab`.
2. "Hosted by Alice Chen (@alice) on lab.example.edu". If your own SSH settings have a name for this server, Crew shows that name instead of the address.
3. The workspace's privacy. "Private" appears with a padlock and the institution, for example "Private · UCSF". "Public" appears as the word alone. If the invitation does not state a privacy, you see "Privacy: The invitation doesn’t say".
4. **Check this invitation (optional)**. See [Check the invitation fingerprint](#check-the-invitation-fingerprint).

If the host or the workspace is not what you expected, choose **Cancel** and contact your host.

### Check your username

The field **Your username on lab.example.edu** is already filled in with the username your host invited. This is your login name on the lab server. It is not your Slack name or your email address.

Change it only if it is wrong. Type it with no spaces, slashes or colons. Crew removes a leading @.

### Check which AI can read the workspace

A line such as "Which AI can read chen-lab: private, UCSF-approved models only" comes from the invitation. You do not need to change it. [Choose which AI can read the workspace](#choose-which-ai-can-read-the-workspace) explains what it means and how to change it.

If the line reads "Choose which AI models may work here: Private or Public.", the invitation does not say. Choose **Private** or **Public** before you continue. The Join button stays unavailable until you do.

### Choose Join

1. Leave **Your agent on lab.example.edu** and **Advanced** as they are, unless your host or IT team told you to change something.
2. Choose **Join chen-lab**. When the invitation names no workspace, the button reads **Join workspace**.
3. The button changes to "Connecting to lab.example.edu…". You cannot close the dialog while it connects.
4. If the server asks for a password or a verification code, the "Sign in to lab.example.edu" window opens by itself. Follow [Sign in to the server](#sign-in-to-the-server).
5. The dialog closes when the connection attempt ends, whether it worked or not.

Choosing Join is your consent to the whole join. When your host lets you in later, Crew finishes joining without asking you again.

If a red note appears at the bottom of the dialog, see [Problems when you choose Join](#problems-when-you-choose-join). If the dialog closes and Crew shows a problem instead of your code, see [Problems while connecting](#problems-while-connecting).

## Send your code to your host

After the dialog closes, Crew shows "Checking your invitation…" for a moment, then a card like this one:

```text
Alice Chen (@alice) invited you to chen-lab.
Send Alice this code:
   7QK2-M9XA-3JTP-WZ4D   [ Copy ]
Waiting for Alice to let you in…
This invitation expires Sat 1:41 AM. You can close Biorouter: Alice can still
let you in with the same code. Next time, open Crew and connect to chen-lab.
Having trouble joining?
```

1. Choose **Copy** next to the code. The button reads "Copied" once the code is on your clipboard.
2. Paste the code into a message to your host, in Slack, email or any other way you reach them.
3. Send the message. Your host enters the code in their Crew.

About your code:

- Your host can type it with or without the dashes, in capitals or in lower case. Crew reads it either way.
- The code belongs to this computer. Reconnecting, disconnecting and closing Biorouter do not change it.
- The code never contains the letters I, L, O or U. If you type it by hand, a round character is a zero.
- The code is not the workspace fingerprint. The workspace fingerprint is in the Join dialog, and you never send it.
- If the button reads "Copy failed", type the code into your message instead.

## Wait for your host

While you wait, Crew shows this:

| Where | What you see |
|---|---|
| The status under the workspace name, at the top of the Crew sidebar | "Not joined yet". Point to it to read "Waiting for Alice Chen (@alice) to let you in". |
| The Crew sidebar | "Your channels appear here once Alice Chen (@alice) lets you in." |
| The workspace menu | **People…**, **Privacy…**, **Agent access…** and **Create team…** are unavailable, with the note "Available after you join". |

- Crew checks with the workspace every 5 seconds while the Crew window is on screen, and again as soon as you return to it.
- The wait depends only on when your host enters your code. It can take minutes or hours.
- You can close Biorouter while you wait.

### Come back after closing Biorouter

What you see depends on whether Biorouter's background service kept running while Biorouter was closed. On a Mac or Linux computer, quitting Biorouter leaves the service running until you restart your computer.

If the service kept running:

1. Open Biorouter. A window titled "Connect to existing BioRouter daemon" asks for your approval secret. Type the secret you set when the service started, and press Return.
2. Choose **Crew**. You do not need to connect again.
3. If your host has not let you in yet, the card with your code is there. The code is the same.
4. If your host let you in while Biorouter was closed, Crew finishes joining by itself, and the workspace opens.
5. If Crew shows another screen instead, follow it. See [Problems while connecting](#problems-while-connecting).

The service stops when your computer restarts. On Windows, it also stops every time you quit Biorouter. If the service stopped:

1. Open Biorouter. On a Mac or Linux computer, set your approval secret in two windows: "Set approval secret for shared BioRouter daemon", then "Confirm shared daemon approval secret". Type the same secret in both. You can use the secret you used before.
2. Choose **Crew**. The page reads "chen-lab is offline". Choose **Connect to chen-lab**.
3. If the server asks for a password or a code, sign in. See [Sign in to the server](#sign-in-to-the-server).
4. If your host has already let you in, Crew finishes joining by itself. If not, the card with your code comes back. The code is the same.

For more, see [When you reopen Biorouter](connections-and-troubleshooting.md#when-you-reopen-biorouter).

### Reconnect and Disconnect while you wait

Both items are in the workspace menu. To open it, choose the workspace name at the top of the Crew sidebar.

| Item | Note under it | What it does |
|---|---|---|
| **Reconnect** | "Try the connection again. Your code doesn’t change." | Connects to the server again. |
| **Disconnect** | "Stop waiting for now. Alice Chen (@alice) can still let you in with the same code." | Closes the connection. Crew does not reconnect by itself. Choose **Connect to chen-lab** when you want to continue. |

## When your host lets you in

1. Your host enters your code in Crew.
2. Your card changes to "Joining chen-lab…" with a spinner. You do not need to click anything.
3. The workspace opens in the same window.

What you see first depends on how your host set things up:

| Situation | What you see | What to do |
|---|---|---|
| Your host added you to a team | The team's channels in the Crew sidebar. A channel shows "Welcome to #{channel}" and "Ask Alice Chen (@alice) to add you to other channels." | Start reading and writing. |
| Your host sent you a team invitation | "You’re invited to {team}" and "Alice Chen (@alice) invited you." | Choose **Join {team}**. |
| You are in no team yet | "You’re in chen-lab" and "Ask Alice Chen (@alice) to add you to a team." | Ask your host to add you, or choose **Create a team**. |
| Your team has no open channel | "No open channels in {team}" and "Create one to start talking." | Choose **Create channel**, or ask your host. |

### Choose your display name

Until you choose a name, other people see your username. After you join, if your account on the server has a full name, a note above the message box offers it, for example "Use “Bob Lee” as your name in chen-lab?". The suggested name comes from that account. Crew does not apply it until you choose one of these:

| Choice | What it does |
|---|---|
| **Use** | Saves the suggested name. Other people see it within a few seconds. |
| **Edit…** | Opens your profile with the name filled in, so you can change it first. |
| **Dismiss** | Closes the note and keeps your username. |

If no note appears, open the You menu at the bottom of the Crew sidebar and choose **Edit profile…** to set your name. See [Set your display name](teams-channels-and-people.md#set-your-display-name).

Your host sees a notice that you joined. You do not get a notice of your own. For more about names, see [Teams, channels and people](teams-channels-and-people.md).

## Choices in the Join dialog

Most people change nothing on this list. Each part says when you need it.

### Check the invitation fingerprint

The fingerprint lets you confirm that the invitation names the same workspace your host runs. Checking it is optional.

1. In the summary, choose **Check this invitation (optional)**.
2. Read the fingerprint. It is four groups of four characters separated by spaces, for example "Fingerprint 6682 327B A040 C709."
3. Ask your host to read the fingerprint from their Crew. It appears in the "Fingerprint" row of their workspace menu.
4. If the two match, continue with Join. If they differ, do not choose Join. Ask your host to send the invitation again.

The fingerprint has no Copy button on purpose. It is not your code, and you never send it. Your code appears only after you choose Join.

### Choose which AI can read the workspace

This setting decides which AI models may read the workspace through your connection:

| Choice | Meaning shown in the dialog |
|---|---|
| **Private** | "Only private and institution-approved models" |
| **Public** | "Public models allowed for public-safe work" |

- It is about AI models. It does not change who can see you, and it does not change what you yourself can read in the workspace.
- The workspace's own setting, which your host controls, decides what is allowed. A Private workspace blocks public models whatever you choose here.
- Your choice is saved on this computer only. Crew does not send it to your host or to anyone else.

For the full explanation, see [Privacy and security](privacy-and-security.md).

To change the choice:

1. Choose **Advanced**.
2. Choose **Change which AI can read chen-lab…**. The choice opens under the heading "Privacy", with the cursor on the selected option.
3. Choose **Private** or **Public**.
4. For Private, type your institution's short ID in **Institution**, for example `ucsf`. It must match the one your host uses. Crew keeps what you typed if you switch to Public and back.
5. Choose **Done**. The choice folds back into its one line. **Done** is available only when the choice is complete: Public, or Private with a valid institution.

If you choose Public for a workspace that is Private, Crew explains what that means:

> "chen-lab is Private, so nothing changes yet: your agent still uses only private and UCSF-approved models here. If Alice makes chen-lab Public, public models could read its public-safe channels through your agent. Alice isn’t told what you chose."

When your choice differs from the invitation in another way, a line under the choice says so, for example "chen-lab is Private for UCSF. Your connection will be Private for sdsc."

Messages about the institution:

| Message | What to do |
|---|---|
| "Private needs an institution. Enter it, or choose Public." | Type the institution's short ID, or choose Public. |
| "Use the short ID: lowercase letters, numbers, - or _, like ucsf." | Retype the ID. It starts with a letter or a number and has at most 64 characters. |
| "Your invitation didn’t include the lab’s institution. Ask Alice which institution chen-lab uses." | Ask your host for the short ID, then type it. |

### Your agent on the server

The row **Your agent on lab.example.edu** controls whether your Biorouter agent may work in a folder on the server. It is off by default, and the folded row reads "Your agent on lab.example.edu is off". Leave it off unless you know you need it. You can change it after you join, in **Connection settings…** in the workspace menu.

To turn it on:

1. Choose **Your agent on lab.example.edu**.
2. In **Remote work folder**, type the full path of a folder on the server, starting with /, for example `/home/bob/project`. Your agent can read and write files there.
3. If your agent should also run commands in that folder, turn on **Let my agent run commands in this folder**. The switch is unavailable until a folder is set, and shows "Add a remote work folder first." Clearing the folder turns the switch off.

Once folded, the row reads "is off", "can use /home/bob/project", or "can use /home/bob/project and run commands there". For what your agent does in Crew, see [Agents and chat access](agents-and-chat-access.md).

### Advanced server connection details

**Advanced** holds settings for how Crew reaches the server. It opens by itself when the invitation names no server, or when your Biorouter background service is too old to read invitations. Otherwise, leave it closed unless your IT team gave you settings.

| Field | Use it when | Rules |
|---|---|---|
| **Change which AI can read chen-lab…** | You want to change the privacy choice. | See [Choose which AI can read the workspace](#choose-which-ai-can-read-the-workspace). |
| **Server login** | Your SSH settings have their own name (an alias) for this server that Crew should use instead of `bob@lab.example.edu`. | Letters, numbers and the characters `_ . / : @ % -`. It must not start with a dash. Required when the invitation names no server. |
| **Port** | Your IT team gave you a port other than the one in the invitation. The usual port is 22. | A number from 1 to 65535. |
| **Identity file** | You sign in with an SSH key file that your SSH settings do not already name. | The file's full path, starting with /. Leave it empty to use your SSH settings. |
| **Jump host** | Your IT team told you to reach the server through a gateway server. | Host names separated by commas, like `gateway.ucsf.edu`. |
| **Connection name** | You want a different name for this workspace on this computer. | At most 120 characters. The default is the workspace name. If another saved workspace already has that name, Crew adds the server to it. |
| **Enter workspace details manually** | See the next section. | |

### Enter workspace details manually

Use this only in two cases: your host gave you workspace details instead of an invitation, or the dialog says "This feature needs a newer Biorouter background service. Quit and reopen Biorouter, or enter the workspace details manually under Advanced." In the second case, quit and reopen Biorouter first, then paste the invitation again.

1. Choose **Advanced**.
2. Turn on **Enter workspace details manually**. The invitation box disappears.
3. Fill in all five fields with the details your host gave you:

   | Field | What to type |
   |---|---|
   | **Your server login** | Your login and the server, for example `bob@lab.example.edu`. |
   | **Socket path** | The full path Crew printed on the server. It starts with /. |
   | **Workspace ID** | The workspace ID, as given. |
   | **Host user ID** | A number, like 1000. |
   | **Workspace key** | 64 characters, using only the digits 0 to 9 and the letters a to f. |

4. Choose **Join workspace**.

## Sign in to the server

Crew signs in to the server with your own account. If your computer has an SSH key the server accepts, you never see this step. Otherwise Crew asks for a password, a verification code, or both.

The "Sign in to lab.example.edu" window opens:

- by itself, when you choose Join or Connect and the server asks for a password or a code;
- when you choose **Sign in** on the screen "Sign in to lab.example.edu";
- when you choose the status "Sign-in needed" under the workspace name (it works as a button);
- when you choose **Sign in…** in the workspace menu.

The window's title always shows the server's address, even where other screens use your own name for the server.

### Type your password or code

The window says "Type your password or verification code in the box below. Nothing you type is saved."

1. The cursor is already in the box. The box shows the server's own prompt, for example a request for your password.
2. Type your password and press Return. The box does not show the characters of a password as you type.
3. If the server asks for a verification code, type the code and press Return.
4. When signing in succeeds, the window closes by itself and Crew continues. You do not need to choose anything else.

**Trouble signing in?**, under the box, says:

- "Use the same username and password you use for this server."
- "If your IT team gave you a jump host, add it in Connection settings."
- "Crew checks servers against this file:", followed by the file's path, usually `~/.ssh/known_hosts`.

### Close the window

**Close** is the only way out of the window. Escape and clicking outside it do nothing. **Close** stops signing in. Crew then shows the screen "Sign in to lab.example.edu" with "The server needs your password or a verification code." Choose **Sign in** to try again.

### Messages in the Sign in window

| Message | What it means | What to do |
|---|---|---|
| "SSH authentication ended (exit {number}). Choose Reconnect to check the connection." | The server ended signing in, for example after a wrong password. | Choose **Close**. Open the workspace menu and choose **Reconnect**. |
| "Your input couldn’t reach the server. Close this sign-in and try again." | Something you typed did not reach the server. | Choose **Close**. Open the workspace menu and choose **Reconnect**. |
| "Authentication refused. Close and reconnect." (inside the box) | Crew could not finish signing in. A common cause is that you signed in but Crew could not start on the server. The workspace menu then shows "Signed in, but Crew couldn't start on the server. Crew may not be set up for your account there." | Choose **Close**, then **Reconnect**. If the same thing happens, see [Crew is not set up for your account](#crew-is-not-set-up-for-your-account). |
| "SSH authentication is already opening." | A second Sign in window was requested while the first was still starting. | Choose **Close**, then open the Sign in window again, once. |
| "Close an existing terminal before opening SSH authentication." | Too many terminals are open in this window. | Close another terminal, then try again. |
| "Signing in needs the Biorouter desktop app." | Crew is open in a web browser, not in the desktop app. | Sign in from the Biorouter desktop app. |

## Add this computer to your existing account

If you are already a member and want Crew on a second computer, the steps are almost the same.

1. Ask your host to invite this computer to your account. When your host invites you, they turn on **Add another device for @bob**.
2. On the new computer, follow [Join from the Biorouter app](#join-from-the-biorouter-app) with the invitation your host sends.
3. The card reads "Alice Chen (@alice) invited this computer to your account in chen-lab." Send your code as usual.
4. After your host lets it in, the card reads "Adding this computer to chen-lab…", then the workspace opens.

## Join with a token on an older server

Some servers run an older version of Crew that cannot let people join with a code. For those, Crew shows a card titled "Join with an invitation token" instead of your code.

1. Under "Send this join request to your host:", copy the join request. It looks like this:

   ```text
   Crew join request for chen-lab
   Username: bob
   Device key: <64 letters and numbers>
   ```

   The device key is a public key. It is not a secret.
2. Send the join request to your host. Your host creates a token from it. A token works once and expires in an hour.
3. Paste the token your host sends into the field **Token from an older invitation**. The field hides what you type. The eye button next to it shows the token.
4. Choose **Join workspace**. The button reads "Joining…" while Crew works.

Your host may also ask for a join request, or send you a token, while your code card is on screen:

1. Choose **Having trouble joining?** under the card.
2. For a join request: after "If Alice asks for a join request:", choose **Show the join request**. Copy the request and send it to your host.
3. For a token: choose **Alice sent me a token instead**. Paste the token into the field that appears, and choose **Join with a token**.

When everything works normally, you do not need **Having trouble joining?**. It starts with "Alice hasn’t let you in yet. That’s normal: Alice lets you in by entering your code in Crew."

If the workspace refuses a token, a red note shows its reason. Ask your host for a new token.

## Join from the command line

People who use a terminal can join with the `biorouter crew` command. Save your host's message in a text file, then run:

```bash
biorouter crew connections join-invitation ./lab-invitation.txt --preview
biorouter crew connections join-invitation ./lab-invitation.txt
biorouter crew --connection chen-lab auth
biorouter crew --connection chen-lab join
```

Each command first asks for your approval secret. Type the secret you set when Biorouter started, and press Return. Nothing appears on the screen as you type.

- When Biorouter's background service is running, the prompt reads `Crew approval secret (printable ASCII, no spaces):`.
- When the service is not running, the prompt reads `Crew approval secret (held separately from your profile):`. The command then starts the service with the secret you type.
- If you type a different secret than the one the service started with, the command fails with "Daemon returned 403: Authorize this action in the Crew panel or native Crew CLI with your human approval secret. Agent tools use their separate task grant." The message does not say that the secret was wrong.

See [The approval secret](command-line.md#the-approval-secret).

After the secret, the commands do this:

1. The first command shows what the invitation says and saves nothing: the workspace, the host, the server, the workspace's privacy, the fingerprint, your username and the privacy this computer will use.
2. The second command asks "Save this connection? [y/N]". Type `y`. It prints "Saved chen-lab." and "Next: biorouter crew --connection chen-lab join".
3. `auth` signs you in to the server, for a password or a verification code. Run it before the first `join`. It needs a terminal where you can type.
4. `join` prints your code, for example "Send Alice this code: 7QK2-M9XA-3JTP-WZ4D", and waits. Pressing Ctrl+C stops waiting and keeps the invitation open. Run `join` again to continue. When your host lets you in, it prints "You're in chen-lab."

Useful options for `join-invitation`:

| Option | What it does |
|---|---|
| `--username` | Your username on the server. The default is the username your host invited. |
| `--mode private` or `--mode public` | Your privacy choice. The default is the workspace's own. |
| `--institution` | The institution for a Private connection. The default is the workspace's. |
| `--name` | This computer's name for the connection. The default is the workspace's name. |
| `--ssh-target` | A login from your own SSH settings, used instead of username@server. The invitation's port and jump host are then not applied. |
| `--port`, `--identity-file`, `--proxy-jump` | The same settings as **Advanced** in the desktop app. An empty `--proxy-jump` means no jump host. |
| `--remote-root`, `--remote-execution` | The remote work folder, and whether your agent may run commands there. |
| `--yes` | Saves without asking. Needed when you pass the invitation on standard input with `-`. |

`join` accepts `--no-wait`, which prints where joining stands and returns at once.

Command line messages:

| Message | What to do |
|---|---|
| "Daemon returned 403: Authorize this action in the Crew panel or native Crew CLI with your human approval secret. Agent tools use their separate task grant." | The approval secret you typed is not the one the service started with. Run the command again and type the right secret. |
| "Paste the whole invitation your host sent, or the brcrew1: line in it." | The file was empty. Save the whole message in it. |
| "Saving this invitation needs {list}." | Add the options the message names, such as `--username` or `--institution`. |
| "Nothing was saved." | You answered no. Run the command again and type `y`. |
| "Add --yes to save this connection; there is no terminal to ask in. Run with --preview to check it first." | Check with `--preview`, then add `--yes`. |
| "Restart the shared Biorouter daemon to invite or join with an invitation." | Quit and reopen Biorouter, then run the command again. |
| "Connect to the workspace first: biorouter crew auth" | Run `auth`, then `join` again. |
| "The code Alice entered doesn't match this computer. Send it again: {code}" | Send your host the same code again. |
| "This invitation expired. Ask {host} to invite you again." | Ask for a new invitation. The command ends with status 1. |
| "This workspace's server can't let people join with a code yet. Ask the host for an enrollment token instead." | Ask your host for a token. See [Command line](command-line.md). |
| "Biorouter reported a join status this version of the command doesn't know. Update Biorouter and try again." | Update Biorouter. |

For every command and option, see [Command line](command-line.md).

## When joining does not work

Find the message you see in the tables below. Messages are grouped by the moment they appear.

### Problems while pasting the invitation

| What you see | What it means | What to do |
|---|---|---|
| "This doesn’t look like a Crew invitation. Ask your host to copy it again." | Crew found no usable invitation in the text. It may be cut off, damaged, or the wrong text. An invitation from a newer version of Biorouter also shows this. | Paste the whole message, including the line that starts with `brcrew1:`. If it still fails, ask your host to copy the invitation again. If your host uses a newer Biorouter, update yours and paste again. |
| "This feature needs a newer Biorouter background service. Quit and reopen Biorouter, or enter the workspace details manually under Advanced." | The Biorouter background service is older than the app. **Advanced** opens with **Enter workspace details manually** turned on. | Quit and reopen Biorouter, then paste again. If that does not help, ask your host for the workspace details and [enter them manually](#enter-workspace-details-manually). |
| "You already have chen-lab on this computer." The Join button is replaced by **Open chen-lab**. | This computer already has this workspace. | Choose **Open chen-lab**. Nothing new is saved, and your code does not change. |
| "This invitation doesn't match “chen-lab”, which this computer already has for the same workspace. Ask your host to send it again, and compare the fingerprint." | The invitation names a workspace this computer already has, with a different identity. Crew never replaces a saved identity from a paste. | Do not join with this invitation. Ask your host to send it again, and [check the fingerprint](#check-the-invitation-fingerprint). **Open chen-lab** opens the workspace you already have. |
| "Privacy: The invitation doesn’t say" in the summary | The invitation does not state the workspace's privacy. | Ask your host whether the workspace is Private or Public, then [choose it](#choose-which-ai-can-read-the-workspace). |
| "This invitation doesn’t name its server. Type your server login here." under **Server login** | The invitation does not include the server's address. | Type your login, for example `bob@lab.example.edu`, or your SSH alias for the server. |

### Problems when you choose Join

If a field needs fixing, a message appears under it. Crew opens the folded section that holds the field and puts the cursor there.

| Message under a field | What to do |
|---|---|
| "Fill this in to continue." | Fill in the field. Fields that must be filled in show "Required" beside their label. |
| "Check this value." | Correct the value. |
| "Private needs an institution. Enter it, or choose Public." | Type the institution's short ID, or choose Public. |
| "Use the short ID: lowercase letters, numbers, - or _, like ucsf." | Retype the institution ID. |
| "Use a server login or SSH alias: letters, numbers and _ . / : @ % -, not starting with a dash." | Retype the server login. |
| "Use a port from 1 to 65535." | Type a port number in that range. |
| A message about the remote work folder | Type a folder path that starts with /. |

If the background service refuses the save, a red note appears at the bottom of the dialog:

| Message | What to do |
|---|---|
| "Type your username on lab.example.edu, with no spaces, slashes or colons." | Correct **Your username on lab.example.edu**. |
| "Type your username on lab.example.edu." | Fill in your username. |
| "@bob can't be used as an SSH login. Set a server login under Advanced instead." | Open **Advanced** and type a **Server login**, for example your SSH alias for the server. |
| "Type a server login from your SSH settings, like hpc or bob@hpc.ucsf.edu." | Correct **Server login**. |
| "This invitation doesn't name its server. Add a server login under Advanced." | Open **Advanced** and type your server login. |
| "Choose the institution for this private workspace, like ucsf." | Type the institution's short ID. |
| "Type an institution as lowercase letters, numbers, - or _, like ucsf." | Retype the institution ID. |
| "Choose a port between 1 and 65535." | Correct **Port**. |
| "Type jump hosts as host names separated by commas, like gateway.ucsf.edu." | Correct **Jump host**. |
| "Choose the identity file by its full path." | Type the key file's full path, starting with /. |
| "Connection names can be at most 120 characters." | Shorten **Connection name**. |
| "This computer already has “chen-lab” for this workspace. Change it in its connection settings instead." with **Open chen-lab** | Choose **Open chen-lab**. To change its settings, use **Connection settings…** in the workspace menu. |
| "Biorouter couldn't read or save this invitation. Try again. If it keeps failing, the Biorouter log has the details." | Choose Join again. If it keeps failing, contact your IT team. |
| "Biorouter couldn't read this request. Update the app or command that sent it, and try again." | Update Biorouter, then try again. |
| "Biorouter sent a saved connection that Crew couldn't read. Retry, or quit and reopen Biorouter." | Try again. If it repeats, quit and reopen Biorouter. |
| "Authorize this action in the Crew panel or native Crew CLI with your human approval secret. Agent tools use their separate task grant." | Crew could not confirm that a person made the request. Choose Join again yourself in the Crew window. If it repeats, quit and reopen Biorouter. |
| "This daemon cannot verify human Crew actions. Start the trusted desktop launcher or biorouter crew daemon start with your separately held approval secret." | The background service cannot confirm your actions. Quit Biorouter and open it again the usual way. If the message stays, contact your IT team. |
| "Crew settings are being saved by another Biorouter process. Try again in a moment." | Wait a few seconds, then choose Join again. |

### Problems while connecting

These appear after the dialog closes, when Crew cannot reach or trust the server. The status under the workspace name shows a short word. The main area shows the details.

| Status or screen | What it means | What to do |
|---|---|---|
| "chen-lab is offline", with "Tried again at 3:04:17 PM. Couldn’t reach lab.example.edu." and "Crew keeps trying by itself while the network is down." A bar at the top reads "Can’t reach lab.example.edu." | Your computer cannot reach the server over the network. | Check your internet connection. If your lab requires a VPN (virtual private network, the app that connects your computer to your institution's network), check that it is on. Crew tries again by itself after 20 seconds, 60 seconds and 3 minutes, then every 5 minutes for an hour. You can also choose **Connect to chen-lab**. |
| "Sign in to lab.example.edu" with "The server needs your password or a verification code." Status: "Sign-in needed". | The server wants a password or a code, and the Sign in window was closed. | Choose **Sign in**. See [Sign in to the server](#sign-in-to-the-server). |
| "Can’t verify lab.example.edu yet". Status: "Can’t verify server". | Your computer does not know this server's identity yet. | See [Crew cannot verify the server](#crew-cannot-verify-the-server). |
| "lab.example.edu’s identity changed". Status: "Can’t verify server". | The server's identity is different from the one your computer knew. | See [The server's identity changed](#the-servers-identity-changed). |
| "This isn’t the workspace you joined". Status: "Can’t verify server". | The server answered for a different workspace than the invitation named. | See [The server answered for a different workspace](#the-server-answered-for-a-different-workspace). |
| "Crew isn’t set up for your account on lab.example.edu". Status: "Not set up on this server". | Crew is not installed for your account on the server. | See [Crew is not set up for your account](#crew-is-not-set-up-for-your-account). |
| "Tried again at {time}. It didn’t connect." or a bar reading "Can’t connect to lab.example.edu." Status: "Can’t connect". | The connection failed for another reason. | Choose **Connect to chen-lab** once more. If it still fails, check **Connection settings…** in the workspace menu with your IT team. |
| "You already use this server for {other workspace} ({institution}). chen-lab uses {institution}; one computer can't mix institutions on the same server." | Another workspace on this computer uses the same server with a different institution. One computer can use a server for one institution only. | Ask your host which institution `chen-lab` uses. For more, see [Privacy and security](privacy-and-security.md). |

#### Crew cannot verify the server

The screen reads "Can’t verify lab.example.edu yet" and "Crew only connects to servers you’ve already verified." It may also show "Fingerprint the server offered", followed by a value that starts with `SHA256:`. That value is the server fingerprint, the same value SSH shows in a terminal. Crew has no button to accept a server. This is on purpose: you confirm the server's identity yourself, once, with the fingerprint from your IT team.

**How do I verify it?** lists three steps:

1. "Get lab.example.edu’s fingerprint from your IT team or your institution’s directory. Check jump hosts too."
2. "Compare it using your usual SSH setup, then add the full key to your known-hosts file. A fingerprint alone isn’t enough."
3. "Come back and choose Try again."

To do these steps, follow [Verify the server on this computer](#verify-the-server-on-this-computer). It gives every command and prompt. **Open a terminal here**, under **How do I verify it?**, opens a terminal inside Biorouter for step 2 of that procedure. When you finish, choose **Try again**. The card with your code appears next. If the server asks for a password or a code first, the "Sign in to lab.example.edu" window opens.

#### The server's identity changed

The screen reads "lab.example.edu’s identity changed" and "Don’t connect until your IT team confirms this change. Crew won’t connect while the old key is in your known-hosts file." It shows "Fingerprint Crew knew" and "Fingerprint the server offered now".

1. Choose **Copy details for IT**. The button reads "Copied". If it reads "Copy failed", the details appear in a box you can copy from.
2. Send the details to your IT team. Do not try to connect in the meantime.
3. When your IT team confirms the change and the old key is gone from your known hosts file, open the workspace menu and choose **Reconnect**.

#### The server answered for a different workspace

The screen reads "This isn’t the workspace you joined" and "The server answered with a different workspace key. Don’t continue until Alice Chen (@alice) confirms what changed."

1. Choose **Copy details** and send the details to your host.
2. Do not continue until your host explains what changed.
3. **Connection settings…** shows the server this workspace uses, so you can check it with your host.

#### Crew is not set up for your account

The screen reads "Crew isn’t set up for your account on lab.example.edu" and "It’s installed once per account, usually by your host or IT team."

1. Copy the message in the box labeled "message to your host". It reads: "Hi Alice, Crew isn’t set up for my account (@bob) on lab.example.edu yet. Could you or IT install biorouter-crew in ~/.local/bin for me?"
2. Send it to your host.
3. When your host or IT team says it is done, choose **Try again**.

**Install it yourself** holds the commands to install Crew in your own account. Use them only with a copy of `biorouter-crew` that your host or IT team gave you. For details, see [Administration](administration.md).

### Problems while you wait

| What you see | What it means | What to do |
|---|---|---|
| "Waiting for Alice to let you in…" for a long time | Your host has not entered your code yet. This is normal. | Remind your host. You can close Biorouter meanwhile. |
| "The code Alice entered doesn’t match this computer. Send it again:" with your code | Your host entered a code that is not this computer's. | Send your host the same code again. It has not changed. |
| "You’re not in chen-lab yet" and "Ask Alice Chen (@alice) to invite @bob. This page updates by itself." | The workspace has no invitation for your username. | Copy the message in the box labeled "message to your host" ("Hi Alice, please invite @bob to chen-lab in Crew.") and send it. The page changes by itself once your host invites you. If @bob is not your login on the server, correct **Your server login** in **Connection settings…**, choose **Save connection**, and connect again. |
| "This invitation has expired." or "This invitation expired. Ask Alice Chen (@alice) to invite you again." | The 24 hours have passed. | Ask your host to invite you again. Keep Crew open on the workspace: the card changes back to your code by itself, and the code is the same. If you paste the new invitation, Crew says "You already have chen-lab on this computer." Choose **Open chen-lab**. |
| "Reconnecting to chen-lab…" | The connection dropped and Crew is reconnecting once by itself. | Wait. |
| "Crew isn’t connected to chen-lab, so it can’t check your invitation." with **Reconnect** | Reconnecting by itself did not help. | Choose **Reconnect**. Sign in if the server asks. |
| "Crew couldn’t check your invitation. It tries again by itself." followed by a reason | Crew could not ask the workspace about your join this time. | Wait. Crew tries again every 5 seconds. If it continues, quit and reopen Biorouter. |

If the card says "Joining chen-lab…" but the join cannot finish, a red note with **Try again** appears under it. The note shows one of these messages:

| Message | What to do |
|---|---|
| "Joining didn’t finish." or "Biorouter couldn't finish joining the workspace. Try again. If it keeps failing, the Biorouter log has the details." | Choose **Try again**. |
| "Your host sent a new invitation. Check your join status and try again." | Choose **Try again**. |
| "Your host hasn't let this computer in yet. Send them the code shown on your screen." | Send your code to your host. |
| "You're not invited to this workspace yet. Ask your host to invite you." | Ask your host to invite you. |
| "This invitation expired. Ask your host to invite you again." | Ask your host for a new invitation. |
| "Your account on the server changed since your host invited it. Ask your host to invite you again." | Ask your host for a new invitation. |
| "Another member of this workspace already has your username. Ask your host to remove the old account first." | Ask your host to remove the old member, then invite you again. |
| "This workspace's server doesn't support joining by invitation yet. Ask your host for an invitation token instead." | See [Join with a token on an older server](#join-with-a-token-on-an-older-server). |
| "This computer's key is already in this workspace." | Tell your host. |
| "The workspace didn't let this computer join." | Tell your host. |

### After you joined

| What you see | What it means | What to do |
|---|---|---|
| "You’re no longer in chen-lab" and "This computer or your account was removed from chen-lab. If you didn’t expect that, ask Alice Chen (@alice)." | Your host removed this computer or your account. Crew does not reconnect by itself. | Ask your host. |
| A bar reading "You’re no longer a member of chen-lab." | The same. | Ask your host. |

For connection problems after you have joined, such as "Offline" or "Updates unavailable", see [Connections and troubleshooting](connections-and-troubleshooting.md).

## Using the keyboard

- The Join dialog opens with the cursor in the invitation box. Tab moves through the fields in order.
- Escape closes the dialog while nothing is running. Clicking outside the dialog does not close it. While Crew saves or connects, the dialog cannot be closed.
- After a paste folds the invitation box, the cursor is on **Edit**. **Edit** returns the cursor to the box.
- **Change which AI can read chen-lab…** moves the cursor to the selected privacy option. The arrow keys choose between Private and Public. **Done** returns the cursor to the Change link, or to **Advanced** if that section is closed.
- Section headings such as **Advanced**, **Check this invitation (optional)** and **Having trouble joining?** are buttons. Press Return or Space to open or close them.
- On the code card, the **Copy** button is named "Copy your code" for screen readers.
- **Alice sent me a token instead** moves the cursor into the token field.

## Related documentation

- [Crew user manual](README.md): the list of every page in this manual.
- [Getting started](getting-started.md): what you need before you use Crew, and the parts of the Crew window.
- [Hosting a workspace](hosting-a-workspace.md): what your host does, including sending the invitation and letting you in with your code.
- [Teams, channels and people](teams-channels-and-people.md): teams, channels and your display name after you join.
- [Messages and files](messages-and-files.md): writing messages and sharing files.
- [Agents and chat access](agents-and-chat-access.md): what your agent can do in Crew, including the remote work folder.
- [Privacy and security](privacy-and-security.md): Private and Public, institutions, keys and fingerprints.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connection status words, reconnecting, and **Connection settings…**.
- [Command line](command-line.md): every `biorouter crew` command and option.
- [Administration](administration.md): installing `biorouter-crew` for each account on the server.
