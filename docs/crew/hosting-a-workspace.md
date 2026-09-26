# Hosting a workspace

> **What this is.** Every task a Crew host does, as numbered steps: starting Crew on the lab server, creating the workspace, inviting people, letting them in, and managing people, names and privacy afterwards.
> **Status:** Current. Checked against the Crew code on 2026-09-25.
> **Audience:** The lab member who hosts a Crew workspace, including people who have never used SSH or a terminal.

A Crew workspace lives on a Linux server that your lab uses, inside one person's account on that server. That person is the host. The host starts Crew on the server once, creates the workspace from Biorouter, and then decides who joins. This page follows those tasks in the order you meet them.

In this manual, a word in braces stands for a name that Crew fills in. For example, "Get {workspace} ready" appears on your screen as "Get wong-lab ready" when your workspace is called `wong-lab`. In the same way, {server} is the server's name as your computer knows it, and {first} is the first name of the person you are inviting.

## What a host does

The host is the person whose server account runs Crew for the workspace, and whose computer created it. Crew shows the host as "Hosted by {name}" in the workspace menu and in Workspace settings. In the member list, the host has a "Host" badge whose tooltip reads "Workspace host: runs {workspace} on the server".

Only the host can do these things:

- invite people to the workspace and let them in;
- cancel an invitation;
- remove a person from the workspace;
- rename the workspace;
- change the workspace privacy and set its institution;
- add members to teams and channels that someone else owns. Other members can add people only to the teams and channels they own.

The server checks each of these actions. The app is not the only check. Only a person at the host's own computer can do them. An agent can never do them, even when it has access to the workspace.

The host also starts Crew again after every server restart. Crew does not start by itself, so members cannot connect until the host, or the IT team, starts it. See [After the server restarts](#after-the-server-restarts).

The tasks, in order:

1. [Check what you need](#before-you-start).
2. [Name the workspace](#name-the-workspace). This is step 1 of 3 in the Host dialog.
3. [Start Crew on the server](#start-crew-on-the-server). This is step 2 of 3.
4. [Create the workspace](#create-the-workspace). This is step 3 of 3.
5. [Mark the workspace with your institution](#mark-the-workspace-with-your-institution).
6. [Set your name and create the first team](#get-the-workspace-ready), from the setup checklist.
7. [Set up Crew in each person's server account](#set-up-crew-for-each-person), before you invite them.
8. [Invite people](#invite-people), [let them in](#let-people-in) and [add them to a team](#add-the-new-person-to-a-team).
9. [Manage the workspace](#manage-the-workspace) as your lab changes.
10. [Keep Crew running on the server](#keep-crew-running-on-the-server), for example after the server restarts.

## Before you start

| You need | Why | If you do not have it |
|---|---|---|
| The Biorouter desktop app on your computer | Crew is part of it. You open Crew from the **Crew** item in the app sidebar. | See [Getting started](getting-started.md). |
| An account on a Linux server your lab uses | The workspace is stored in this account. | Ask your IT team for a server login. |
| A server Crew can run on | The released Crew program is made for Linux servers with an x86_64 processor and glibc 2.31 or newer. No other processor type has a tested build. Crew keeps the workspace in your home folder, so that folder must be on the server's own local disk, and neither your group nor any other account may write to it. The server name you sign in with must always reach the same machine, because members can reach Crew only on the machine where it runs. | Send your IT team two questions: "Is my home folder on local disk?" and "Does this server name always reach the same machine?" If the name can reach several machines, ask for the name of one machine and type that name as your server login in step 1 of the Host dialog. [Check a server before you host](administration.md#check-a-server-before-you-host) in Administration lists commands that answer these questions. |
| SSH access from this computer to that account | Biorouter reaches the server over SSH. For **Start it for me**, the server must accept this computer's SSH key without asking for a password or a code. To find out whether it does, see [Check which way you will start Crew](#check-which-way-you-will-start-crew). | You can still host. Run the start commands yourself, as described in [Run the commands yourself](#run-the-commands-yourself). |
| The server's fingerprint, a value that starts with `SHA256:` | The first time this computer signs in to the server, SSH shows the server's fingerprint. You compare the two to confirm that you reached the real server. It is not the workspace fingerprint. | Ask your IT team. Your institution's directory may list it too. |
| This computer has signed in to the server before | Crew connects only to servers whose identity this computer already trusts. | Follow steps 1 to 3 of [Check which way you will start Crew](#check-which-way-you-will-start-crew). After you check the server's fingerprint and type `yes`, this computer trusts the server. |
| Crew installed in your server account, at `~/.local/bin/biorouter-crew` | This is the Crew program that runs on the server. | See [Install Crew on the server](#install-crew-on-the-server). If you have not used a terminal before, ask your IT team to install it in your account. |
| Crew installed in each member's server account, at `~/.local/bin/biorouter-crew` | Biorouter starts Crew in each person's own account when that person connects. | See [Set up Crew for each person](#set-up-crew-for-each-person). |
| Your institution's short ID, for example `ucsf` | A Private workspace needs it. It decides which AI models agents may use. | In step 1 of the Host dialog, Crew fills in "Institution" by itself when the models set up in your Biorouter name exactly one institution. Biorouter also names the institution beside an institutional model when you choose a model, for example "UCSF", and Crew shows it as "Private · UCSF". For UCSF, the short ID is `ucsf`. Ask your IT team or research office only if neither place shows it. |

Words this page uses:

| Word | Meaning |
|---|---|
| SSH | The secure way your computer signs in to the lab server over a network. |
| SSH key | A matching pair of keys that lets SSH sign you in without a password. The private key stays on your computer. The server keeps the public key for your account. IT often sets this up for you. |
| Server login | The account name and server address you sign in with, such as `alice@hpc.example.edu`. It can also be a short name for the server from your SSH settings, such as `lab-server`. |
| SSH settings | The SSH configuration on your computer. IT often sets it up for you. It can give a server a short name, called an alias. |
| Institution ID | Your organization's short ID: lowercase letters, numbers, `-` or `_`, such as `ucsf`. |
| Workspace fingerprint | Sixteen letters and numbers in four groups, such as `6682 327B A040 C709`. People you invite can ask you to read it out, to check that their invitation came from your workspace. |
| Code | Sixteen letters and numbers that a person's Biorouter shows after they paste your invitation, such as `7QK2-M9XA-3JTP-WZ4D`. They send it to you, and you enter it to let them in. The code is not the fingerprint. |
| Connection | What Biorouter saves on your computer to reach a workspace: the server login, the privacy choice and a few server settings. |

### Check which way you will start Crew

Do this once, before you open the Host dialog. It tells you whether **Start it for me** can sign in for you, and it makes this computer trust the server.

1. Open a terminal on your computer. On a Mac, press Command and Space, type Terminal, and press Return.
2. Type `ssh` followed by your server login, such as `ssh alice@hpc.example.edu`, and press Return.
3. If the terminal asks "Are you sure you want to continue connecting", compare the `SHA256:` fingerprint it shows with the one from your IT team. Type `yes` and press Return only if they match. If they do not match, type `no`, press Return, and contact your IT team. [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer) shows the exact lines the terminal prints and the answers it accepts.
4. Look at what happened next:
   - You are signed in and never typed a password or a code. **Start it for me** can sign in for you.
   - The server asked for a password or a verification code. Plan to use **Run it yourself in a terminal** in step 2 of the Host dialog.
5. Leave the server:
   - If the terminal shows the server's prompt, type `exit` and press Return.
   - If the terminal is waiting for a password or a code, press Control and C. Do not type `exit` there, because the server would read it as a wrong password.

## Open the Host dialog

Use either of these:

- On the first Crew screen, "Work together in Crew", choose **Host a new workspace**. It is the link under the **Join a workspace** button.
- If you already have a workspace, open the workspace menu by choosing the workspace name at the top of the Crew sidebar. Then choose **Add a workspace**, then **Host a new workspace…**.

The "Host a new workspace" dialog opens. Its header shows the three steps, "Name", "Start" and "Create". A finished step shows a check mark.

The dialog starts empty each time you open it. The one exception is a workspace you started but did not finish creating. See [Finish a workspace you did not create](#finish-a-workspace-you-did-not-create).

## Name the workspace

This is step 1 of 3, "Name". The cursor starts in "Workspace name".

1. In "Workspace name", type a name for the workspace, such as `Wong Lab`. Crew turns what you type into a workspace name and shows it under the field: "Your workspace: wong-lab".
2. In "Your server login", type the login IT gave you, such as `alice@hpc.example.edu`. You can type the short name from your SSH settings instead, such as `lab-server`.
3. Under "Privacy", keep **Private**, or choose **Public**. Read [What the Privacy choice in step 1 sets](#what-the-privacy-choice-in-step-1-sets) first.
4. If Private is chosen, check "Institution". Crew fills it in when the model providers set up in your Biorouter name exactly one institution. Otherwise, type your institution's short ID, such as `ucsf`.
5. Optional: to let your agent work in a folder on the server, choose **Your agent on {server}** and fill in "Remote work folder". While this row is folded it reads as one sentence, such as "Your agent on lab-server is off". See [Agents and chat access](agents-and-chat-access.md).
6. Optional: choose **Advanced** only if IT gave you a port, an identity file or a jump host. While it is folded, "server connection details" appears beside it.
7. Choose **Continue**, or press Enter. The button reads "Preparing…" while Biorouter prepares this computer's hosting identity. This is a key pair that lets this computer, and no other, create the workspace.

After Continue, the dialog moves to step 2 and calls the server by the name you typed in step 1 until it closes.

### Fields in step 1

| Field | Rules | Notes |
|---|---|---|
| "Workspace name" | Required. 1 to 40 characters after Crew converts it: lowercase letters a to z, numbers and dashes, starting and ending with a letter or number. | Crew removes accents, changes capitals to lowercase, and turns spaces and other characters into dashes. "Wong Lab" becomes `wong-lab`. |
| "Your server login" | Required. | A login such as `alice@hpc.example.edu`, or an alias from your SSH settings. |
| "Privacy" | **Private** (the default) or **Public**. | Private reads "Only private and institution-approved models". Public reads "Public models allowed for public-safe work". |
| "Institution" | Required while Private is chosen. 1 to 64 characters: lowercase letters, numbers, `_` and `-`, starting with a letter or number. | The helper reads "Your organization’s short ID. People who join as Private use the same one." The value stays if you switch to Public and back. |
| "Remote work folder" | Optional. A full path on the server that starts with `/`. | The helper reads "Optional. A folder on {server}, starting with /. Your agent can read and write files there." |
| "Let my agent run commands in this folder" | Off unless you turn it on. | It stays unavailable until you enter a folder, with the hint "Add a remote work folder first." Clearing the folder turns it off. |
| "Port" (under Advanced) | 1 to 65535. | Leave it empty unless IT gave you a port. |
| "Identity file" (under Advanced) | Optional. | "Leave empty to use your SSH config." |
| "Jump hosts" (under Advanced) | Optional. | Fill it in only if IT gave you a jump host. |
| "Connection name" (under Advanced) | Optional. | Left empty, the connection is named after the workspace. |

### Messages in step 1

A message appears under the field it is about, and the cursor moves to the first field with a problem. If that field is inside a folded section, Crew opens the section first. The message goes away as soon as the value is right.

| Message | What to do |
|---|---|
| "Fill this in to continue." | Fill in the field. |
| "Use at least one letter or number." | The name has no letters or numbers left after conversion. Type a name that has some. |
| "Private needs an institution. Enter it, or choose Public." | Type your institution ID, or choose **Public**. |
| "Use the short ID: lowercase letters, numbers, - or _, like ucsf." | Type the short ID only, such as `ucsf`, not the full name of your organization. |
| "Use a port from 1 to 65535." | Fix the port, or clear the field. |

### What the Privacy choice in step 1 sets

The Privacy choice in step 1 sets your own connection on this computer. It does not set the workspace.

- Every new workspace starts "Private for everyone", with no institution, whatever you choose here.
- Choosing Public in step 1 does not make the workspace Public. To allow Public in the workspace later, see [Allow Public in the workspace](#allow-public-in-the-workspace).
- The institution you type here is saved with your connection. After you create the workspace, Crew asks whether to set the same institution for the workspace itself. See [Mark the workspace with your institution](#mark-the-workspace-with-your-institution).

Private and Public decide which AI models may read the workspace. They do not decide which people can see it. Only people you let in can see the workspace. See [Privacy and security](privacy-and-security.md).

## Start Crew on the server

This is step 2 of 3, "Start". The heading reads "Start Crew on {server}". The line under it reads "These commands start Crew on {server} as {user}:", and a box shows four commands:

```bash
umask 077
mkdir -p "$HOME/.local/share/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/{slug}" --name {slug} --bootstrap-key {key}
"$HOME/.local/bin/biorouter-crew" status --state-dir "$HOME/.local/share/biorouter-crew/{slug}"
```

In the box, `{slug}` is replaced by your workspace name, and `{key}` by this computer's public hosting key, 64 characters long. The key is public. It is not a secret.

What each line does:

1. `umask 077` makes the files that the next commands create readable only by your account.
2. `mkdir -p` creates the folder in your account where Crew keeps its workspaces.
3. `start` starts Crew for this workspace. The key means only this computer can create the workspace.
4. `status` prints the workspace's details, which Biorouter reads.

Long lines scroll sideways in the box. Scroll right to see the end of a line. The **Copy** button copies all four lines.

At the bottom of step 2, the dialog notes: "Anyone who can sign in to this server can see the workspace name." Choose the name with that in mind, and keep patient or sample IDs out of it.

You can run the commands in two ways. **Start it for me** is the usual way.

### Start it for me

1. Choose **Start it for me**.
2. Wait. The dialog shows "Starting Crew on {server}…" and then "Reading what Crew printed…". The box "What the server printed" shows the output as it arrives.
3. When Crew has started, the dialog moves to step 3 by itself, with the cursor on **Create workspace**. In testing this took about 4 seconds.

The hint under the button says what it does: "Biorouter signs in to {server} as {user} with your SSH settings and runs exactly these commands. If the server asks for a password or a code, run them yourself instead."

In more detail:

- Biorouter signs in with your SSH settings, as the login from step 1, using the port, identity file and jump hosts from Advanced.
- It runs the commands shown, character for character. Nothing you typed elsewhere in the dialog becomes part of a command.
- It never answers a password or code prompt. If the server asks for one, Start it for me stops, and you run the commands yourself.
- If the commands Biorouter is about to run differ from the ones shown, it stops at once and says so.
- It stops a run that takes longer than 180 seconds.
- It uses only your own login. It gains no other account or permission on the server.

To stop a run, choose **Stop**. Closing the dialog does not stop the commands on the server. It only stops the dialog from following them.

### If Start it for me reports a problem

When something goes wrong, the dialog shows a warning with one sentence, and the section "Run it yourself in a terminal" opens below it.

| Message | What to do |
|---|---|
| "Crew was still starting. Wait a few seconds, then choose Start it for me again." | Wait a few seconds and choose **Start it for me** again. |
| "Crew isn’t installed on {server} yet. Open “Crew isn’t on {server} yet?” below." | Install Crew in your server account. See [Install Crew on the server](#install-crew-on-the-server). |
| "Crew on the server said: {detail}", where the detail contains `unsupported_storage` | Your home folder is on a network file system (NFS, CIFS or SMB). Crew cannot keep a workspace there. Send the whole message to your IT team. See [Storage](administration.md#storage) in Administration. |
| "Crew on the server said: {detail}", where the detail contains `unsafe_storage` | The workspace folder, a file in it, or a folder above it fails Crew's safety checks, for example because other accounts can open or write to it. When the detail ends in `writable non-sticky ancestor`, your group or other accounts can write to your home folder or a folder above it. Send the whole message to your IT team. See [Storage](administration.md#storage) in Administration. |
| "Crew on the server said: {detail}", where the detail contains `node_identity_unavailable` | The server has no usable machine ID file, `/etc/machine-id`. Only the server's administrator can add it. Send the whole message to your IT team. See [Machine identity](administration.md#machine-identity) in Administration. |
| "Crew on the server said: {detail}", with any other detail | Crew on the server reported an error. Read the detail. If it is unclear, send it to your IT team. |
| "{server} didn't accept this computer's SSH key for {login}. Check the server login, or run the commands yourself in a terminal." | Choose **Back** and check "Your server login". If the login is right, run the commands yourself. |
| "The server asks for a password or a code, so Biorouter can't sign in for you. Run the commands yourself in a terminal." | Run the commands yourself. |
| "This computer hasn't connected to the server before. Sign in once in a terminal to check its fingerprint, then try again." | Follow steps 1 to 3 of [Check which way you will start Crew](#check-which-way-you-will-start-crew). Then choose **Start it for me** again. |
| "The server's identity changed since this computer last connected. Check with the server's administrator before you continue." | Do not continue. Contact your IT team before you do anything else. |
| "Couldn't reach the server. Check the login and your network, then try again." | Check the login and your network connection, then choose **Start it for me** again. |
| "SSH couldn't run the commands. Run them yourself in a terminal to see why." | Run the commands yourself. |
| "Biorouter couldn’t find what Crew prints in the output. Run the commands yourself to see what the server says." | Run the commands yourself. |
| "Biorouter stopped because the commands it was about to run weren’t the ones shown here. Run them yourself instead." | Run the commands yourself. |
| "Starting Crew didn’t finish. Run the commands yourself to see what the server says." | Run the commands yourself. |
| "Starting Crew took too long, so it was stopped. Run the commands yourself to see what the server says." | Run the commands yourself. |
| "Stopped. Crew may have started on the server; run the commands yourself to check." | You chose **Stop**. Run the commands yourself to see where things stand. |
| "Crew is already being started on a server from this computer. Wait for it to finish." | Wait for the other run to end. |
| "Start it for me needs a newer Biorouter background service. Quit and reopen Biorouter, or run the commands yourself." | Quit and reopen Biorouter, then try again. |
| "This host setup already has a saved connection. Open it from Crew." | Close the dialog and open the workspace from Crew. If Crew shows a "Finish creating {workspace}" card, see [Finish a workspace you did not create](#finish-a-workspace-you-did-not-create). |
| "This computer has no host setup with that ID. Start hosting again." | Close the dialog and open **Host a new workspace** again. |

### Run the commands yourself

Use this way when the server asks for a password or a code, or when Start it for me reports a problem. Choose **Run it yourself in a terminal** to open it. The text inside reads "Run the commands above in a terminal signed in to {server} as {user}, then paste what they printed."

1. If you do not have a terminal signed in to the server, copy the sign in command under "Not signed in to the server in a terminal yet? Sign in first:". It looks like `ssh alice@hpc.example.edu`.
2. Choose **Open a terminal here** to open a terminal inside the dialog. You can use your own terminal app instead. The terminal types and runs nothing by itself. Choose **Hide terminal** to close it.
3. Paste the sign in command into the terminal and press Enter.
4. If the terminal asks you to confirm the server, compare the fingerprint it shows with the one from your IT team. Type `yes` only if they match.
5. If the server asks for your password or a verification code, type it in the terminal.
6. Choose **Copy** beside the start commands, paste them into the terminal, and press Enter.
7. Select everything the commands printed, copy it, and paste it into "Paste what it printed".
8. Wait a moment. Crew reads the paste by itself and shows "Checking what you pasted…". When it finds the workspace, it shows "Found {workspace} on {server}" with a check mark.
9. Choose **Continue**. The button reads "Reading…" while Crew reads the paste.

You do not need to trim the paste. Crew finds its own lines and ignores your prompt and other output around them. If the terminal broke a long line in two, Crew joins it again. Reading the paste saves nothing.

If you press Enter while this section is folded, Crew opens it and asks for the paste there.

### If the paste is not accepted

| Message | What to do |
|---|---|
| "That isn’t what Crew prints. Copy everything after the command ran and paste again." | Copy all the output that appeared after you pressed Enter, and paste again. |
| "Crew was still starting when this was printed. Wait a few seconds, run the last command again and paste what it prints." | Wait a few seconds, run the last line (the `status` command) again, and paste what it prints. |
| "The paste stops partway through what Crew printed. Copy the whole line, from { to }, and paste again." | Copy the whole line, from the opening `{` to the closing `}`, and paste again. |
| "Crew isn’t installed on {server} yet. Open “Crew isn’t on {server} yet?” below." | See [Install Crew on the server](#install-crew-on-the-server). |
| "Crew on the server said: {detail}", where the detail contains `unsupported_storage`, `unsafe_storage` or `node_identity_unavailable` | The server cannot hold the workspace as it is set up now. See the rows for these details in [If Start it for me reports a problem](#if-start-it-for-me-reports-a-problem). |
| "Crew on the server said: {detail}", with any other detail | Read the detail. It is Crew's own error message from the server. If it is unclear, send it to your IT team. |

### Enter the workspace details by hand

In two cases, the dialog asks you to type the workspace details yourself:

- "This feature needs a newer Biorouter background service. Quit and reopen Biorouter, or enter the workspace details below." Quit and reopen Biorouter first. If the message comes back, fill in the fields.
- "Biorouter read the workspace but not every detail it needs. Enter the rest from what the commands printed." The fields already hold what Crew could read.

Copy each value from what the commands printed:

| Field | What it holds |
|---|---|
| "Socket path" | A full path that starts with `/`. |
| "Workspace ID" | The workspace's ID. |
| "Host user ID" | A number, such as `1000`. |
| "Workspace key" | 64 characters, using the digits 0 to 9 and the letters a to f. |

Then choose **Continue**.

### Install Crew on the server

Crew runs on the server as a program called `biorouter-crew`. It is installed once in each server account, at `~/.local/bin/biorouter-crew`. Installing it does not need administrator rights on the server.

If you have not used a terminal before, ask your IT team, or whoever manages Biorouter for your lab, to install `biorouter-crew` in your server account at `~/.local/bin/biorouter-crew`. When they are done, open the Host dialog, or go back to it, and choose **Start it for me** in step 2.

You can install it before you open the Host dialog. This section prints every command you need.

To install it yourself, first find out whether the server already has Crew. Sign in to the server in a terminal and run `command -v biorouter-crew`. If it prints a path, such as `/usr/bin/biorouter-crew`, your IT team installed Crew for the whole server. Skip to step 6 and use that path. If it prints nothing, start at step 1.

The steps below use the Terminal app on a Mac. To open it, press Command and Space, type Terminal, and press Return. On another kind of computer, ask your IT team for the file.

1. On your computer, open the Biorouter release page: <https://github.com/BaranziniLab/biorouter/releases>.
2. Under the newest release, in its list of files, download `biorouter-cli_<version>_amd64.deb`. Your browser saves it in your Downloads folder.
3. Check that the download is intact. In a terminal on your computer, run this command with the real file name:

   ```bash
   shasum -a 256 ~/Downloads/biorouter-cli_<version>_amd64.deb
   ```

   Compare the long value it prints with the `sha256:` value shown beside that file on the release page. If the two differ, delete the file and download it again. Do not install a file whose values differ.
4. Copy the package to the server. In the same terminal, run this command with the real file name and your own server login:

   ```bash
   scp ~/Downloads/biorouter-cli_<version>_amd64.deb alice@hpc.example.edu:
   ```

   The package lands in your home folder on the server.
5. Sign in to the server with `ssh alice@hpc.example.edu`. Then take the Crew file out of the package:

   ```bash
   dpkg-deb --extract biorouter-cli_<version>_amd64.deb staging
   echo "$HOME/staging/usr/bin/biorouter-crew"
   ```

   The first command installs nothing. It puts the Crew file in a folder called `staging` in your home folder. The second command prints the file's full path, such as `/home/alice/staging/usr/bin/biorouter-crew`. Keep this path for step 6. If the server says `dpkg-deb: command not found`, the server cannot open this package. Ask your IT team for the file.
6. In the terminal that is signed in to the server with your own login, type this line with your own path between the single quotes, and press Return. Use the path from step 5, or the path `command -v biorouter-crew` printed. The line prints nothing. It keeps the path under the name `VERIFIED_BINARY` for the next commands.

   ```bash
   VERIFIED_BINARY='/home/alice/staging/usr/bin/biorouter-crew'
   ```

7. Copy these three lines, paste them into the same terminal, and press Return:

   ```bash
   mkdir -p "$HOME/.local/bin"
   install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"
   "$HOME/.local/bin/biorouter-crew" --version
   ```

8. The last line prints the version of Crew. That shows the install worked.
9. Open the Host dialog, or go back to it, and choose **Start it for me** in step 2.

The Host dialog shows the same four commands under **Crew isn’t on {server} yet?** in step 2. There, the first line holds a placeholder path, `/replace/with/path/to/verified/linux/biorouter-crew`, and its **Copy** button copies all four lines with the placeholder. Type the first line yourself, as in step 6, and copy only the last three lines.

[Get a verified file](administration.md#get-a-verified-file) in Administration lists the other sources of the file, including the `.rpm` package and a build from source.

## Create the workspace

This is step 3 of 3, "Create". The heading reads "{workspace} on {server}".

1. If the dialog says "Your SSH settings call this server {label}.", your SSH settings have a short name for the address you typed. Both names mean the same server. From now on, Crew calls the server by the short name.
2. Look at "Fingerprint". This is the workspace fingerprint. The dialog explains: "People you invite may ask you to read this to check their invitation. Your workspace menu shows it too." Choose **Copy** if you want to keep a copy.
3. Read the line "Creating {workspace} makes this computer its first admin device."
4. Choose **Create workspace**. The button reads "Creating…" while Crew works.
5. If the server asks for a password or a verification code, the Sign in window opens by itself, and the button reads "Waiting for you to sign in…". Type your password or code in the Sign in window. Nothing you type there is saved.

Create does four things, in this order:

1. It saves the connection on this computer.
2. It connects to the server.
3. It makes this computer the workspace's first admin device.
4. It waits until Crew has checked the workspace's identity.

Next, if Crew knows your institution, the dialog asks you to mark the workspace with it. See [Mark the workspace with your institution](#mark-the-workspace-with-your-institution). Otherwise, the dialog closes and the setup checklist appears.

If something goes wrong:

- "Sign-in didn’t finish. Choose Create workspace to try again." Choose **Create workspace** again.
- If the connection fails, the reason appears in the dialog. Fix the cause, then choose **Create workspace** again. [Connections and troubleshooting](connections-and-troubleshooting.md) explains each reason.

The left button in step 3 is **Back** until Crew has saved the connection. After that it is **Cancel**.

In Keys and security (in the You menu at the bottom of the Crew sidebar), this computer is listed as added "when the workspace was created".

### Finish a workspace you did not create

If you close the dialog after Create saved the connection but before it finished, Crew remembers the setup on this computer. Crew then shows a card:

- Title: "Finish creating {workspace}"
- Text: "Crew is running on the server. Create the workspace to become its first admin."

To finish:

1. Choose **Finish creating…** on the card.
2. The Host dialog opens at step 3. Choose **Create workspace**.

## Mark the workspace with your institution

After Create, the dialog asks this question when three things are true: the workspace is Private, it has no institution yet, and Crew knows your institution. The title reads:

"Mark {workspace} as a {institution} workspace?"

It explains:

1. "Step 1 set {institution} for your connection on this computer. This sets it for {workspace} itself, for everyone who works there."
2. "Agents working in {workspace} can then use only models approved for {institution}. This can’t be undone."
3. "Until then, people can chat and share files in {workspace}, but agents can’t work there. You can do this later from Get {workspace} ready, or from Privacy… in the workspace menu."

This is the second time Crew asks for your institution, and the two answers do different things. Step 1 set your own connection on this computer. This question sets the workspace itself, for every member.

Choose one:

- **Mark as {institution}** sets it now. You cannot change or remove it later.
- **Not now** leaves it for later. Pressing Escape does the same.

What the institution changes:

| Workspace | People | Agents |
|---|---|---|
| Private, no institution | Can chat and share files. | Cannot work in the workspace at all. |
| Private, with an institution | Can chat and share files. | Can work only with models approved for that institution, or with a local model on the person's own computer. |

Once set, the workspace institution cannot be changed or cleared. Crew refuses any later change. To use a different institution, host a new workspace.

The question shows the institution as it reads, such as "UCSF". The later places below show the short ID, such as `ucsf`. Both mean the same institution.

### Mark it later

If you chose Not now, three places offer it again. Each opens the same confirmation.

- The setup checklist row "Confirm the institution", while the workspace has no team: **Set institution to {institution}…**.
- A note above the message box in a channel, titled "Mark {workspace} as a {id} workspace?", with **Set institution to {id}…** and **Not now**. Not now hides the note for this workspace on this computer. Workspace settings still offers it.
- Workspace menu, **Privacy…**, then the "Institution" row: **Set institution to {id}…**.

To confirm:

1. Choose one of the buttons above. Crew asks "Set {workspace}’s institution to {id}?" and explains "This can’t be changed later. Private data can then be used only with models approved for {id}."
2. Choose **Set {id} permanently**, or **Cancel**. **Cancel** is selected when the confirmation opens, so pressing Enter cancels.

If your own connection has no institution, these places say "Add your institution in Connection settings first." To add one:

1. Choose **Connection settings…**.
2. Choose **Private**, fill in "Institution", and choose **Save connection**.
3. Come back and set the workspace institution.

Setting the institution counts as a workspace privacy change. It ends every agent's current access in the workspace. See [Changes that end agent access](#changes-that-end-agent-access).

## Get the workspace ready

When the Host dialog closes, the main area shows the setup checklist, titled "Get {workspace} ready". Work through the rows from the top, so that you set your name before you create the team.

Creating the first team closes the checklist, including its last row, "Invite people". Crew opens the team's `#general` channel in its place. After that, you invite people from the welcome block at the top of the channel ("No one else has joined {workspace} yet.") or from the workspace menu. See [After the first team exists](#after-the-first-team-exists). Before you invite anyone, [set up Crew in each person's server account](#set-up-crew-for-each-person).

If your SSH settings call the server by another name, the checklist says so: "{workspace} is on {address}, which your SSH settings call {label}."

The rows, in order. A finished row shows a check mark.

| Row | Buttons | Finished when |
|---|---|---|
| "Your name: Use “{name}”?" | **Use** and **Edit…** | You have chosen a name. The row then reads "Your name: {name}". |
| "Confirm the institution" | **Set institution to {institution}…**, or **Connection settings…** when your connection has no institution | The workspace has an institution, or it allows Public. In a workspace that allows Public, the row reads "Not needed while the workspace allows Public." |
| "Create a team" | **Create team** | Any team exists. |
| "Invite people" | **Invite people…** | Someone else has joined, or someone is waiting to join. |

The buttons are unavailable while Crew is still checking the workspace.

### Set your name before you invite anyone

The invitation carries your name as it is when you create the invitation. If you invite first, the person sees only your username, such as `@crew_henry`, and an invitation you already sent does not change.

The name row appears when your server account has a full name and your Crew name is still your username.

1. In the row "Your name: Use “{name}”?", choose **Use** to use the name on your server account. To change it first, choose **Edit…**, which opens Edit profile with the name filled in.
2. The row changes to "Your name: {name}", with a check mark.

Crew never sets your name without asking. If you created a team before you answered, the same offer appears above the message box in a channel: "Use “{name}” as your name in {workspace}?", with **Use**, **Edit…** and **Dismiss**. You can also change your name at any time from the You menu, with **Edit profile…**.

### Create a team

A new workspace has no team, and people need a team to see channels.

1. Choose **Create team** in the checklist. You can also choose **Create team…** in the workspace menu.
2. The "Create team" dialog opens with one field, "Name". Type the team's name, such as `Analysis Lab`. The helper under the field reads "Team names are unique in {workspace}."
3. Choose **Create team**, or press Enter.

A team name keeps the capitals and spaces you type. It can be up to 64 characters and needs at least one letter or number. It can use letters, numbers, spaces and these marks: `- _ . ' & ( ) +`. It cannot contain `@`, `#`, `/` or `:`.

What you see next:

- If other people have already joined the workspace, the dialog asks "Add people to {team}". In "Person", choose one person and choose **Add**, or choose **Skip for now**. While you are the only person in the workspace, this step does not appear.
- The dialog closes. The team appears in the sidebar, and Crew opens its `#general` channel. Every team gets `#general` when it is created, and you own it.
- At the top of `#general`, the welcome block reads "Welcome to #general". While you are the only person in the workspace, it also shows "No one else has joined {workspace} yet." with **Invite people to {workspace}…**.

[Create a team](teams-channels-and-people.md#create-a-team) in Teams, channels and people covers teams in full, including every message about team names.

### After the first team exists

The checklist is gone, and its open rows move to other places:

- While you are the only person in the workspace, the channel's welcome block shows "No one else has joined {workspace} yet." with **Invite people to {workspace}…**. This goes away once someone joins or is waiting to join.
- The workspace menu has **Invite people to {workspace}…**, before and after anyone joins. See [Invite people](#invite-people).
- Above the message box, Crew shows one note at a time. First comes the institution note, while the workspace is Private and has no institution (see [Mark it later](#mark-it-later)). After you answer it, the name offer appears, if you have not chosen a name yet.

## Set up Crew for each person

Crew runs in each person's own server account, including yours. When a person connects, Biorouter signs in to the server as that person and starts `~/.local/bin/biorouter-crew` in their account. A copy anywhere else on the server, such as `/usr/bin` or `/usr/local/bin`, does not count. If the file is missing, the person's Biorouter shows "Crew isn’t set up for your account on {server}", and they cannot join.

Set this up before you send the invitation. Which way depends on whether Crew is installed for the whole server. To check, run `command -v biorouter-crew` in a terminal signed in to the server. It prints a path, such as `/usr/bin/biorouter-crew`, when Crew is installed for the whole server. It prints nothing when it is not.

### If Crew is installed for the whole server

The person, or your IT team for them, runs these commands in a terminal signed in to the server as that person:

```bash
mkdir -p "$HOME/.local/bin"
install -m 0755 "$(command -v biorouter-crew)" "$HOME/.local/bin/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" --version
```

The last line prints the version of Crew. That shows the install worked. These are the same commands the Invite dialog shows under **If {first} sees “Crew isn’t set up”**.

### If Crew is not installed for the whole server

1. The person gets the Crew file into their own server account the same way you did: steps 1 to 5 of [Install Crew on the server](#install-crew-on-the-server), with their own server login in place of yours.
2. In a terminal signed in to the server as themselves, they type this line with the full path that step 5 printed for them between the single quotes, and press Return:

   ```bash
   VERIFIED_BINARY='/home/bob/staging/usr/bin/biorouter-crew'
   ```

3. They copy these three lines, paste them into the same terminal, and press Return:

   ```bash
   mkdir -p "$HOME/.local/bin"
   install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"
   "$HOME/.local/bin/biorouter-crew" --version
   ```

4. The last line prints the version of Crew. That shows the install worked.

Your IT team can do these steps for the person instead. A person who has not used a terminal before should ask them to. The person's own Biorouter shows the same four commands under **Install it yourself** on the "Crew isn’t set up" screen. There, the first line holds a placeholder path, and **Copy** copies it too, so the person types the first line with their own path and copies only the last three lines. [Install commands](administration.md#install-commands) in Administration has the full reference.

## Invite people

You invite each person by their login on the server. One invitation is for one person.

Open the Invite dialog from any of these places:

- The workspace menu: **Invite people to {workspace}…**. It is available once the connection is verified. Until then, the menu says why, for example "Available once the connection is verified".
- The setup checklist: **Invite people…**.
- The workspace menu, **People…**, then **Invite people…**.
- The Add people dialog, when no one else has joined yet: **Invite people to {workspace}…**.

Then:

1. In "Username", type the person's login on the server, such as `crew_jack`. The field already shows the "@". If you paste a name that starts with @, Crew removes the extra one.
2. Choose **Invite**, or press Enter.
3. Check the first line of the result. It names the person as the server knows them, for example "@crew_jack · Jack Moreno (name on the server account) · invited".
4. Under "Send {first} this invitation:", choose **Copy** to copy the invitation message. A long message shows four lines and "The whole message is copied."
5. Send the message to the person by email, Slack or any other way you usually reach them.
6. Note the last line: "When {first} sends you a code, choose Let in… next to their name in the sidebar. This invitation expires {day and time}."
7. Choose **Invite another** to invite the next person, or **Done**.

Clicking outside the Invite dialog does not close it, so a stray click cannot lose the message. If the message does not load, the dialog shows "The invitation message couldn’t be loaded." with **Retry**.

### What to type as the username

The username is the person's login on the server, not their name. The helper under the field shows your own login as an example: "The name they sign in to {server} with; yours is @{your username}." In testing, a host typed `bob` and Crew refused it, because the account was `crew_bob`. If you are not sure, ask the person for the exact name they use to sign in to the server.

Crew checks the name on the server when you choose Invite. It looks up only that one name. It does not look anything up while you type, and it never lists the server's accounts.

### If Crew refuses the username

The message appears under the field. Changing the field clears it.

| Message | What it means | What to do |
|---|---|---|
| "No account named @{name} on this server." | The server has no account with that name. | Check the login with the person. |
| "@{name} is already in {workspace}." | The person is already a member. A switch appears: "Add another device for @{name}". | To let them join from another computer, turn on the switch and choose **Invite**. Otherwise, nothing is needed. |
| "Invite @{exact name} instead: that’s the account’s exact name." | The name differs in capitals, or is another name for the same account. | Type the exact name shown and choose **Invite**. |
| "Type the account's name on the server, not its numeric user ID." | You typed a number. | Type the login name. |
| "Type the account's name on the server, with no spaces, slashes, colons or invisible characters." | The name has characters a login cannot have. | Retype the login. |
| "@{name} is a system account on this server and can't join a workspace." | Accounts that belong to the server itself, such as `root` and `nobody`, cannot join. | Invite the person's own account. |
| "Another account on this server is already invited as @{name}. Cancel that invitation first." | A different account is already waiting under that name. | Cancel the other invitation (see [Cancel an invitation](#cancel-an-invitation)), then invite again. |
| "Another active member is already @{name}. Remove the old @{name} first." | A current member already uses that name. | Remove the old member first. See [Remove someone from the workspace](#remove-someone-from-the-workspace). |
| "This account joined as @{old} and is now @{new} on the server. Remove @{old} first." | The account was renamed on the server after it joined. | Remove @{old}, then invite @{new}. |
| "100 people are already waiting to join. Cancel an invitation or wait for one to expire." | The waiting list is full. | Cancel an invitation, or wait for one to expire. |
| "This account's name on the server can't be used to join by invitation. Invite it with an enrollment token instead." | This account cannot join by name. | Use [Invite someone with an older Biorouter](#invite-someone-with-an-older-biorouter). |

If you turn on "Add another device" for someone who is not a member yet, Crew refuses. Turn the switch off and invite them again.

### The invitation message

The message has three lines:

```text
Join {workspace} on Crew.
In Biorouter, open Crew, choose Join a workspace, and paste this whole message.
brcrew1:…
```

The person pastes the whole message. Their Biorouter finds the `brcrew1:` line itself.

- The message holds no secret. If it reaches the wrong person, it gains them nothing. They would also need the account you invited and your approval of their code.
- It carries the workspace name and key, the server's address and port, any jump hosts, your username, your name if you set one, the workspace privacy and institution, and the invited username.
- It carries the server's real address, even when your SSH settings call the server by a short name.
- It expires 24 hours after you create it.
- Inviting the same person again replaces the earlier invitation. Their Biorouter then tells them the host sent a new invitation.
- An expired invitation stays in the list, marked expired, until the next change of any kind in the workspace clears it.

### If the person sees “Crew isn’t set up”

Every person needs Crew installed in their own server account. See [Set up Crew for each person](#set-up-crew-for-each-person). If it is missing, their Biorouter shows "Crew isn’t set up for your account on {server}".

The Invite dialog keeps the commands for this case:

1. In the invitation result, choose **If {first} sees “Crew isn’t set up”**.
2. It reads "Send this to whoever runs {server}, to run in @{username}’s account:" and shows:

   ```bash
   mkdir -p "$HOME/.local/bin"
   install -m 0755 "$(command -v biorouter-crew)" "$HOME/.local/bin/biorouter-crew"
   "$HOME/.local/bin/biorouter-crew" --version
   ```

3. Choose **Copy**, and send the commands to your IT team or to whoever runs the server.

These commands work as written only when Crew is installed for the whole server. They copy the server's file into the person's own account. When Crew is not installed for the whole server, `command -v biorouter-crew` finds nothing and the `install` line fails. Use the steps in [If Crew is not installed for the whole server](#if-crew-is-not-installed-for-the-whole-server) instead.

After the install, the person chooses **Try again** on their "Crew isn’t set up" screen.

### Invite someone with an older Biorouter

Use this only when the person's Biorouter, or Crew on your server, is too old to join by username and code.

1. In the Invite dialog, choose **Other ways to invite (older Biorouter)**.
2. Paste the join request the person sent you into "Their join request".
3. In "Their user ID on {server}", type the person's user ID number. To find it, run the command shown under "Run on the server:", which is `id -u {username}`, in a terminal signed in to the server.
4. Choose **Create invitation**.
5. Choose **Copy** beside the invitation token and send it to the person. The dialog notes: "It works once and expires in an hour."

If the join request has no device key, the dialog says "This join request has no device key. Ask them to copy it again."

## Let people in

After the person pastes your invitation and chooses Join, their Biorouter shows a code of 16 letters and numbers, such as `7QK2-M9XA-3JTP-WZ4D`. They send the code to you, and you enter it to let them in.

Their computer works out the code from its own key and the workspace key. The workspace later admits only the computer whose key gives that same code. So a code you enter lets in that one computer and no other.

While you wait, the sidebar lists the person under "Waiting to join", with the line "Let in… when {first} sends a code".

1. When the person sends you the code, choose **Let in…** beside their name in the sidebar. You can also open the workspace menu, choose **People…**, and choose **Let in…** under "Waiting to join".
2. The dialog "Let {name} into {workspace}" opens, with the cursor in "Code from {first}".
3. Paste or type the code. The helper reads "Paste the code {first} sent you. Only use a code that came from {first}."
4. Choose **Let {first} in**. The button works only when the field holds a whole code.
5. The dialog shows "Code saved. {first} is in as soon as {first}’s Crew checks in; you can close this."
6. When the person's computer checks in, the note turns green: "{first} joined {workspace}". A notification with the same words also appears. In testing this took from 1 to 6 seconds while the person's Biorouter was open.

"Code saved" does not mean the person has joined. Crew compares the code only when the person's computer checks in. If you mistyped the code, you find out then, as a different code warning. You do not need to keep the dialog open.

### How the code field reads a code

- You can paste the code with dashes, spaces, lowercase letters or no separators. Crew removes spaces and dashes, changes letters to capitals, and shows the code in four groups.
- The letters I and L count as the number 1, and the letter O counts as 0.
- A code never contains the letter U. Crew refuses a U rather than guess.

| Message | What to do |
|---|---|
| "A device code has 16 letters and numbers." | The code is too short or too long. Copy it again from the person's message. |
| "Device codes never contain the letter U. Check the code." | Check the code with the person. |
| "A device code has only letters and numbers." | Remove any other characters. |

### The fingerprint in the Let in dialog

The dialog shows "Fingerprint {first} should see:" with the workspace fingerprint, and "If {first} asks, read this out. It should match what {first}’s Crew shows." It also shows when the invitation expires: "{first}’s invitation expires {day and time}." or "{first}’s invitation expired."

The fingerprint and the code are different things. The person sends you the code. You read the fingerprint aloud only if the person asks for it. The workspace menu also shows the fingerprint, with a **Copy** button, while the connection shows "Connected".

### Add the new person to a team

Let in offers to add the person to your teams in the same dialog. Do it here. A person in no team sees no channels. Their Crew shows "You’re in {workspace}" and "Ask {host} to add you to a team."

The buttons depend on how many teams Crew can offer:

- One team: the footer shows **Not now** and **Add to {team}**. **Add to {team}** is the default button.
- Several teams: each team has its own button, **Add {first} to {team}**. The footer shows **Not now**, which changes to **Done** once you have added the person to one team.
- No team to offer: only **Done**.

To add the person:

1. Under "Channels in {team}", tick the channels to add them to. "#general" is always ticked and marked "comes with the team". Channels you own start ticked. Other channels in the team that you can see start unticked, and you can tick them. Choosing a channel's name also ticks its box.
2. Wait until the person has joined. Until then the team button does nothing, and the dialog says "You can add {first} to a team once {first} joins." After they join, it says "Ticked channels are added with the team."
3. Choose **Add to {team}**, or **Add {first} to {team}**.
4. The dialog confirms, for example "Added {first} to {team}. {first} can now see #general and #methods." If the person was already in the team, it says "{first} is already in {team}."

If Crew on your server is older, the buttons read **Invite to {team}** or **Invite {first} to {team}**, and the result reads "Invited. {first} will see it in Crew and needs to accept."

When you let in a second computer for an existing member, **Done** is the default button and extra channels start unticked.

### If you closed Let in before adding them to a team

The sidebar section "Joined, not in your teams" lists people who have joined and are in none of your teams, for example "Jack Moreno (@crew_jack) · joined".

1. Choose **Add to a team…** on the person's row.
2. With one team, the Add people dialog opens for that team. With several, choose the team from the menu.
3. Tick the channels and choose **Add**.

The row disappears as soon as the person is in a team.

### If a computer showed a different code

The Let in dialog shows this warning:

"A computer trying to join as @{username} showed a different code. Check the code @{username} sent you, then enter it and choose Replace code. Don’t approve a code you didn’t get from @{username}."

The person's row in the sidebar shows the same warning in slightly different words: "A computer trying to join as @{username} showed a different code. Check the code @{username} sent you; if you typed it wrong, let them in again with the right code. Don’t approve a code you didn’t get from @{username}."

It means one of two things. You mistyped the code, or a computer other than the person's tried to join as them. From your side, the two look the same.

1. Compare the code you entered with the code the person sent you. If you are not sure, ask the person directly.
2. Choose **Let in…** on their row in the sidebar. In a Let in dialog that is still open, choose **Enter the code again**.
3. Enter the code the person sent you.
4. Choose **Replace code**.

Never approve a code you did not get from the person themselves. Crew never replaces a saved code unless you choose Replace code.

If you enter a new code without choosing Replace code, Crew says "You already entered a code for {first}, and it didn’t match {first}’s computer. Enter the code {first} sent you and choose Replace code." Under it: "Replace the code only if {first} sent you a new one." and a **Replace code** button.

The different code warning is not saved. If Crew restarts on the server, the warning disappears, and nothing else changes.

If Crew says "@{username} has no pending invitation. Invite them first.", the invitation expired or was cancelled. Invite the person again.

## Track who is waiting to join

The sidebar section "Waiting to join" appears only for the host. Each row shows:

- first line: "@{username}", with the row's button;
- second line: the name on their server account, the state and the expiry, for example "Jack Moreno (name on the server account) · invited · expires Sat 1:41 AM";
- third line, while the person is invited: "Let in… when {first} sends a code".

| State | Meaning | Button |
|---|---|---|
| "invited" | You invited the person. No code has been entered. | **Let in…** |
| "Code entered" | You saved a code. Crew checks it when their computer checks in. | None. If a different code was tried, **Let in…** appears again so you can fix it. |
| "Invitation expired" | 24 hours have passed. The person can no longer be let in. | **Invite again…** |

The buttons are unavailable while Crew is reconnecting to the workspace.

Workspace settings shows the same people. Open the workspace menu and choose **People…**. The "Waiting to join" section lists each person with **Cancel invitation** and **Let in…**, or "Invitation expired · Invite again…".

### Cancel an invitation

1. Open the workspace menu and choose **People…**.
2. Under "Waiting to join", choose **Cancel invitation** on the person's row.
3. Crew asks "Cancel @{username}’s invitation?". Choose **Cancel invitation** to cancel it, or **Keep** to leave it. **Keep** is selected first.

### Invite again after an invitation expires

1. Choose **Invite again…** on the person's row.
2. The Invite dialog opens empty. Type the username again and choose **Invite**.
3. Send the new message. The old one no longer works.

## Add people to teams and channels

As host, you can add members of the workspace to teams and channels that someone else owns. The app shows you only the teams and channels you are a member of, so those are the ones you can add people to from the app.

Adding needs nothing from the person. They are in as soon as it finishes, because they agreed to take part when they joined the workspace. A person must be in a team before you can add them to one of its channels.

| Where | What it adds to | Who sees it |
|---|---|---|
| A team's options menu (**⋯** beside the team name in the sidebar): **Add people to {team}…** | The team, its `#general`, and the channels you tick under **Also add to** | The team's owner and the host |
| The sidebar section "Joined, not in your teams": **Add to a team…** | The same as the team's options menu | The host |
| The Let in dialog: **Add to {team}** or **Add {first} to {team}** | The same as the team's options menu. See [Add the new person to a team](#add-the-new-person-to-a-team). | The host |
| A channel's menu: **Add people…** | That channel | The channel's owner only |
| The Members tab in a channel's details: **Add people…** | That channel | The channel's owner only |

### Add people to a channel you do not own

**Add people…** in a channel's menu and on its Members tab appears only for the channel's owner. As host, you add people to someone else's channel while you add them to its team:

1. In the sidebar, point at the team, choose **⋯**, and choose **Add people to {team}…**.
2. Tick the people to add. The list shows only people who are not in the team yet.
3. Under **Also add to**, tick the channel. `#general` is always ticked. Channels you own start ticked. The team's other open channels that you are a member of are listed unticked.
4. Choose **Add**, or **Add {n} people** when you ticked more than one.

To add someone who is already in the team to a channel you do not own, use the command line. The channel must be one you are a member of:

```bash
biorouter crew members add @bob --channel '#methods'
```

If Crew on your server is older, only a team's or channel's owner can add people, and the person gets an invitation to accept. [Teams, channels and people](teams-channels-and-people.md#add-people-to-a-team) covers the Add people dialog in full.

## Manage the workspace

Workspace settings holds most of the host's controls. Open the workspace menu by choosing the workspace name at the top of the Crew sidebar. Then choose **People…**, **Privacy…** or **Agent access…**. Each opens "{workspace} settings" on that tab. These items are unavailable until the connection is verified, and the menu says why.

| Tab | What the host finds there |
|---|---|
| **General** | "Hosted by", "Server" (the short name and the address, with **Copy**), and **Rename…**. |
| **People** | "Waiting to join" and "Members", with **Invite people…**. |
| **Privacy** | "Your connection", "Workspace" and "Institution", with the host's privacy buttons. |
| **Agent access** | Your own chats and tasks that have access in the workspace. See [Agents and chat access](agents-and-chat-access.md). |

Use the arrow keys to move between tabs. Press Escape or choose **Done** to close.

The workspace menu also shows the workspace name, "Hosted by {name}", "Signed in as @{username} on {server}", the connection status and, while connected, the fingerprint with a **Copy** button.

### Rename the workspace

1. Open Workspace settings and choose the **General** tab.
2. Choose **Rename…**.
3. In "Name", type the new name exactly as it should appear: lowercase letters, numbers and dashes, up to 40 characters, starting and ending with a letter or number. Unlike the Host dialog, this field does not convert what you type.
4. Choose **Rename**, or press Enter.

| Message | What to do |
|---|---|
| "Workspace name can use lowercase letters a-z, numbers and hyphens, and must start and end with a letter or number." | Fix the name. |
| "Another workspace you host on this server is already using this name. Choose a different name." | Choose another name. |

A rename keeps the workspace's history, invitations and agent access. The folder on the server keeps the original name. **Rename…** appears only when Crew on your server supports renaming.

### Remove someone from the workspace

Removing a person ends their membership on every computer they use.

1. Open the workspace menu and choose **People…**.
2. Under "Members", point at the person's row and choose the options button (three dots) at its end.
3. Choose **Remove from {workspace}…**.
4. Crew asks "Remove {name} (@{username}) from {workspace}?" and explains "Removes all of their devices and agent access. Their messages stay in history."
5. In "Type {username} to confirm", type the person's username exactly, with the same capital and small letters. The username is shown beside the field.
6. Choose **Remove from {workspace}**. Pressing Enter in the field does not confirm.

If the letters do not match exactly, Crew says "Type the exact username to remove access." and removes nobody.

What happens next:

- The person can no longer use the workspace from any computer. Their Biorouter shows "You’re no longer in {workspace}" and "This computer or your account was removed from {workspace}. If you didn’t expect that, ask {person}."
- Their messages stay in history, where Crew names them as a former member.
- Any pending invitation for their account is removed.
- Every agent's current access in the workspace ends, for every member. See [Changes that end agent access](#changes-that-end-agent-access).

The member options menu also has **Copy username** and **Copy for support**, which holds **Copy person ID**.

Removing a person from one channel is a different action, done by the channel's owner. See [Teams, channels and people](teams-channels-and-people.md).

## Change the workspace privacy

Only the host changes the workspace privacy. A new workspace is "Private for everyone". To see the setting, open the workspace menu and choose **Privacy…**. The "Workspace" row shows "Private for everyone" or "Allows Public".

| Workspace setting | What it means |
|---|---|
| "Private for everyone" | No public AI model can read the workspace through Crew, whatever each member chooses for their own connection. |
| "Allows Public" | Each member may make their own connection Public. Public models can then read the "Public-safe" channels that member can see. "Restricted" channels and messages stay private. |

Neither setting changes which people can see the workspace. [Privacy and security](privacy-and-security.md) explains the whole model, including each member's own connection.

The host's buttons in this tab are unavailable until the connection is verified. If Crew has not finished checking, it says "Crew is still checking this workspace’s privacy. Try again in a moment." Wait, then try again.

### Allow Public in the workspace

1. Open the workspace menu and choose **Privacy…**.
2. In the "Workspace" row, choose **Allow Public…**.
3. Crew asks "Allow Public in {workspace}?" and explains "Members will be able to choose Public. Agents with access will need permission again."
4. In "Type {workspace} to confirm", type the workspace name. Capitals and spaces at either end do not matter.
5. Choose **Allow Public**. Pressing Enter in the field does not confirm.

### Make the workspace Private for everyone

1. Open the workspace menu and choose **Privacy…**.
2. In the "Workspace" row, choose **Make Private for everyone…**.
3. Crew asks "Make {workspace} Private for everyone?" and explains "Agents with access will need permission again."
4. Choose **Make Private for everyone**.
5. A notification confirms: "{workspace} is now Private for everyone. Agents with access need permission again."

To set the workspace institution, see [Mark the workspace with your institution](#mark-the-workspace-with-your-institution).

## Changes that end agent access

Some workspace changes end every agent's current access in the workspace, for every member. After such a change, a chat that had access says "Crew settings changed since access was granted. Grant access again from Crew." Its owner then grants access again.

These changes end agent access:

- removing a person from the workspace;
- adding a person to a team or a channel;
- a person accepting an invitation to a team or channel;
- removing a person from a channel;
- archiving a channel;
- offering or accepting ownership of a channel;
- any change to the workspace privacy or institution.

Renaming the workspace does not end agent access.

## What a host cannot do

- Hand the host role to someone else. Crew has no way to transfer it. **Transfer ownership…** applies to channels only.
- Be removed from the workspace.
- Change or clear the workspace institution once it is set.
- Transfer, rename or archive a channel that someone else owns. Only a channel's current owner can do that. The host can still add people to it. See [Add people to a channel you do not own](#add-people-to-a-channel-you-do-not-own).
- Let an agent invite people, let them in, add them to teams or change privacy. Crew refuses these actions from any agent.
- Start, stop or approve another person's agent, or act as another person.
- See another person's password, verification codes or device keys. These stay on that person's computer.

## What hosting means for your server account

- The workspace is stored in your server account, in `~/.local/share/biorouter-crew/{name}`, where {name} is the name the workspace had when you hosted it.
- Your server account, and any software running as your account, can read everything the workspace stores, including "Restricted" channels and files. Crew does not encrypt channels against the host account. Anyone with administrator rights on the server can read it too.
- The workspace history is one file that only grows. Each change records who made it, what changed and when. Nothing is deleted, including the history of removed people and archived channels. The app has no screen that shows this file.
- A workspace can hold 16 MiB of current data and 1 GiB of history. Shared files count separately; see [Messages and files](messages-and-files.md). When the workspace is full, people can still read it, but changes are refused with "This workspace has grown past the size Crew supports and cannot take more changes. Ask the host about starting a new workspace."
- People reach the workspace through Crew running in your account on the server. [Administration](administration.md) explains how to check, stop and start it.

## Keep Crew running on the server

The workspace runs on the server, in your account. People can use it while your computer is off or Biorouter is closed. Only the tasks that need the host, such as letting someone in, wait until you are back at your computer.

### After the server restarts

Crew installs no service on the server, so nothing starts it again after the server restarts. Until you start it, members cannot connect. Their Crew shows "{workspace} is offline", and choosing **Connect to {workspace}** ends with "Tried again at {time}. It didn’t connect."

If you have not used a terminal before, send this section to your IT team. Crew must run as your own account on the server, so ask them to run the commands as your account, not as an administrator account.

To start Crew again:

1. Open a terminal on your computer. On a Mac, press Command and Space, type Terminal, and press Return.
2. Type `ssh` followed by your server login, such as `ssh alice@hpc.example.edu`, and press Return. If the server asks for your password or a verification code, type it and press Return. The terminal does not show the characters as you type.
3. Run `ls "$HOME/.local/share/biorouter-crew"`. It lists one folder for each workspace you host. Each folder has the name the workspace had when you hosted it, even if you renamed the workspace later.
4. Run this command, with the folder name in place of `{name}`:

   ```bash
   "$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/{name}"
   ```

5. Check what it prints. A line that contains `"state":"running"` means Crew is running. If the line contains `"state":"starting"` instead, wait a few seconds, then run the `status` command:

   ```bash
   "$HOME/.local/bin/biorouter-crew" status --state-dir "$HOME/.local/share/biorouter-crew/{name}"
   ```

6. Tell members to choose **Connect to {workspace}** on the offline screen. Do the same in your own Biorouter.

Do not add `--name` to the `start` command. If you renamed the workspace, `start` refuses the old name. Starting again keeps everything: the same workspace, members, keys and history. [After a server restart](administration.md#after-a-server-restart) in Administration has more detail, including how to stop Crew.

## Keyboard use in host dialogs

| Dialog | Behavior |
|---|---|
| Host a new workspace | The cursor starts in "Workspace name". Enter submits the current step. Escape closes the dialog, except while Crew is working. Clicking outside the dialog does not close it. After **Continue**, the cursor is on **Start it for me**. After a successful start, it is on **Create workspace**. |
| Invite people | Enter invites. After the result appears, the cursor is on **Done**. **Invite another** puts the cursor back in the empty field. |
| Let in | The cursor starts in the code field. Enter submits only a whole code, and otherwise says what is wrong. After the code is saved, the cursor is on **Add to {team}** or **Done**. |
| Workspace settings | Arrow keys move between tabs. Escape closes. |
| Confirmations (Allow Public, Remove from workspace) | **Cancel** is selected when the confirmation opens. Pressing Enter in the typed field does nothing. The confirm button stays unavailable until the typed name matches. To confirm, choose the confirm button, or move to it with Tab and press Space. |
| Set institution | There is no field to type in. **Cancel** is selected when the confirmation opens, so pressing Enter cancels. To confirm, choose **Set {id} permanently**, or move to it with Tab and press Space. |
| Cancel invitation | **Keep** is selected first. |

## Do the same from the command line

Every host task in this page also has a `biorouter crew` command. Add `--connection {name}` when more than one connection is saved. [Command line](command-line.md) covers each command in full.

| Task | Command |
|---|---|
| Prepare this computer's hosting identity | `biorouter crew connections prepare` |
| Create the workspace after Crew started on the server | `biorouter crew --connection lab workspace bootstrap` |
| Set the workspace institution | `biorouter crew --connection lab privacy set-workspace private --institution ucsf` |
| Invite someone | `biorouter crew --connection lab enroll invite @bob` |
| Print an invitation message again | `biorouter crew --connection lab connections invitation --for @bob` |
| See who is waiting to join | `biorouter crew --connection lab enroll pending` |
| Let someone in | `biorouter crew --connection lab enroll approve @bob 7QK2-M9XA-3JTP-WZ4D` |
| Replace a code you already entered | `biorouter crew --connection lab enroll approve @bob 7QK2-M9XA-3JTP-WZ4D --replace` |
| Cancel an invitation | `biorouter crew --connection lab enroll cancel @bob` |
| Remove someone from the workspace | `biorouter crew --connection lab enroll revoke @bob` |
| Add someone to a team and a channel | `biorouter crew --connection lab members add @bob --team "Analysis Lab" --channel '#methods'` |
| Rename the workspace | `biorouter crew --connection lab workspace rename new-name` |

The command line changes the workspace privacy without a typed confirmation, unlike the desktop app.

## Related documentation

- [Crew user manual](README.md): the list of every page in this manual.
- [Getting started](getting-started.md): what you need before you start, and the parts of the Crew window.
- [Joining a workspace](joining-a-workspace.md): what the people you invite see and do.
- [Teams, channels and people](teams-channels-and-people.md): teams, channels, adding and removing people, and channel ownership.
- [Agents and chat access](agents-and-chat-access.md): what agents can do in the workspace, and granting access again.
- [Privacy and security](privacy-and-security.md): Private and Public, institutions, fingerprints and keys.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connection statuses, signing in and server identity problems.
- [Command line](command-line.md): every `biorouter crew` command.
- [Administration](administration.md): installing Crew on the server, where it keeps data, and its limits.
