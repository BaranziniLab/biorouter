# Hosting a workspace

> **What this is.** The host's tasks as numbered steps: setting Crew up and starting it on the lab server, creating the workspace, choosing who manages it, letting people in, managing it afterwards, and deleting it.
> **Status:** Current. Checked against the Crew code on 2026-10-16.
> **Audience:** The lab member who hosts a Crew workspace, including people who have never used SSH or a terminal.

A Crew workspace runs in the host's account on a Linux server. Biorouter installs Crew's server program in your account and starts the workspace for you, so you need no terminal. Braces mark a name Crew fills in, such as {workspace}, {server}, or {first}, the first name of the person you invite.

## What only the host can do

Crew labels you "Hosted by {name}" in the workspace menu and "Host" in the member list. Whatever the workspace's governance, only you remove people from the workspace, rename it, change its privacy, institution and governance, and delete it. The server accepts these actions only from a person at your computer, never from an agent.

A workspace is **open** or **managed** ([Choose who manages the workspace](#choose-who-manages-the-workspace)). A workspace you create is open: every member can rename, archive and delete teams and channels, add people to them, join any of them, invite people to the workspace and let them in. In a managed workspace, a team's creator and a channel's owner manage them, and only you invite people and let them in. [Roles and permissions](teams-channels-and-people.md#roles-and-permissions) has the whole table.

Like every member, you see only the teams and channels you are in. In an open workspace that is every team and channel, because a new member is added to all of them and a new team or channel gets everyone. In a managed workspace, a team or channel another member creates does not appear in your sidebar or in `biorouter crew` until someone adds you, so you cannot add anyone to it. Its owner does that.

The workspace runs on the server, so people can use it while your computer is off. After every server restart, you start Crew again ([After the server restarts](#after-the-server-restarts)).

The workspace is stored in your account, in `~/.local/share/biorouter-crew/` under its original name. Your account and the server's administrators can read all of it ([What other people can see](privacy-and-security.md#what-other-people-can-see)).

## Before you start

| You need | Details |
|---|---|
| Biorouter on your computer | See [Installation and setup](../getting-started/installation.md). |
| A login on a Linux server Crew can run on | Ask IT whether the server is 64 bit Intel or AMD Linux (x86_64) with glibc 2.31 or newer, whether your home folder is on local disk and writable only by you, and whether its name always reaches the same machine. If not, use one machine's name. See [Check a server before you host](administration.md#check-a-server-before-you-host). |
| SSH access | Biorouter signs in with this computer's SSH key, or with your password or code, which you type in its Sign in window. |
| The server's fingerprint | A value that starts with `SHA256:`, from IT. Follow [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer) once, with your own login, so this computer trusts the server. |
| Your institution's short ID, such as `ucsf` | Agents need it in a Private workspace. A model shown as "Private · UCSF" means `ucsf`. Otherwise, ask IT. |

You do not need Crew on the server yet. **Set up and start Crew** installs it in your account at `~/.local/bin/biorouter-crew`, without administrator rights, from the copy built into Biorouter. Each member's Biorouter does the same in their own account when they join.

## Host the workspace

On the first Crew screen, choose **Host a new workspace**. If you already have a workspace:

1. Choose its name at the top of the sidebar.
2. Choose **Add a workspace**.
3. Choose **Host a new workspace…**. The dialog "Host a new workspace" opens.

The dialog has three steps: Name, Start and Create. To do the same from a terminal in one command, see [Host a workspace](command-line.md#host-a-workspace).

### Name the workspace

1. In **Workspace name**, type a name, such as `Wong Lab`. Crew shows it as "Your workspace: wong-lab". Anyone who can sign in to the server can see it, so leave out patient and sample IDs.
2. In **Your server login**, type your login, such as `alice@hpc.example.edu`, or the server's short name from your SSH settings.
3. Under **Privacy**, keep **Private** or choose **Public**. With Private, type your short ID in **Institution** if it is empty.
4. Check **Your agent on {server}**. It starts at **As me on the server**, which lets your agent run commands as your account there, with the warning that goes with it. Choose **In a folder** or **Off** to give it less ([What your agent may do on the server](agents-and-chat-access.md#what-your-agent-may-do-on-the-server)). As host, your account also holds the workspace's own files, so full access lets your agent read channels you are not in.
5. Fill in **Advanced** only if IT gave you a port, an identity file or a jump host.
6. Choose **Continue**. Biorouter makes a hosting key, so only this computer can create the workspace. The dialog moves to step 2, Start.

The Privacy choice sets your own connection. Every new workspace starts "Private for everyone". With **Private**, **Create workspace** in step 3 also marks the workspace with the institution you typed, permanently ([Create the workspace](#create-the-workspace)).

### Set up and start Crew

Step 2 shows the commands that make a private folder, start Crew and print its details. The key in them is public. To change the name, choose **Back**.

1. Choose **Set up and start Crew**.
2. If the server asks for a password or a code, the Sign in window opens. Type it there. One sign-in covers the whole setup and **Create workspace** after it.
3. Watch the steps: "Checking server", "Signing in", "Installing Crew in your account", "Starting workspace" and "Ready". Beside the install step, Crew says "Installed Crew {version}." or "Crew {version} was already installed."
4. When Crew has started, the dialog moves to step 3 by itself.

Biorouter runs exactly the commands shown, as you, plus fixed commands that check the server and install Crew. It sends Crew's server program over the SSH connection itself, checks its SHA-256 on the server, and installs it with one rename, keeping any older copy beside it as `biorouter-crew.previous`. It never replaces a newer copy with an older one, and keeps an older copy that still runs, which `biorouter crew setup --replace-existing` updates ([Upgrade Crew](administration.md#upgrade-crew)). A copy of the same version that is not byte for byte the one Biorouter carries is replaced, and Crew says which copy it replaced. **Stop** ends the setup. Closing the dialog does not stop the commands on the server.

#### Fix folder permissions

Crew keeps a workspace only where other accounts cannot change the folders above it. If a folder in your home on the way to `~/.local/share/biorouter-crew`, such as `~/.local`, can be changed by your group or by everyone, setup stops before it starts the workspace. The message names each folder and the command that fixes it, `chmod go-w` followed by the folders.

- Choose **Fix it for me**. Biorouter runs `chmod go-w` on exactly the folders named, and only on folders inside your home that your account owns, then continues the setup.
- Or run the command yourself on the server, then choose **Set up and start Crew** again.

The change only stops other accounts from writing into those folders. They can still read and enter them as before. From a terminal, `biorouter crew host` refuses the same way and suggests `--fix-permissions` ([Host a workspace](command-line.md#host-a-workspace)).

#### Run the commands yourself

Use this when **Set up and start Crew** cannot reach the server, or fails.

1. Choose **Run it yourself in a terminal**, then **Open a terminal here**, or use your own terminal.
2. Paste the `ssh` command shown there into the terminal, and press Enter. Check the fingerprint if asked, and type your password or code.
3. If Crew is not in your server account yet, do [Install Crew yourself](#install-crew-yourself) first.
4. Choose **Copy** beside the start commands, paste them into the terminal, and press Enter.
5. Copy all the output, including the whole line from `{` to `}`, into **Paste what it printed**.
6. When Crew shows "Found {workspace} on {server}", choose **Continue**. The dialog moves to step 3, Create.

#### If setting up does not work

The message says what failed, and **What the server printed** holds the server's own words, for IT. **Run it yourself in a terminal** stays available.

| Message starts with | What to do |
|---|---|
| "Crew is installed in your account, but it didn’t start the workspace" | Open **What the server printed**. For `unsafe_storage` or `unsupported_storage`, see [Storage requirements](administration.md#storage-requirements), and for `node_identity_unavailable`, [Machine identity](administration.md#machine-identity). Otherwise send the text to IT. |
| "Other people on {server} can change your home folder" | Run `chmod go-w ~` on the server, then try again. |
| "Your home folder on {server} is on network storage" | Host from an account whose home is on a local disk, or ask IT. |
| "The Crew already in your account on {server} doesn’t work with this Biorouter" | Choose **Replace the installed Crew**. The old copy is kept beside it. |
| "Your account on {server} can’t write to ~/.local/bin" or "There isn’t enough free space" | Free space or fix the folder, or ask IT. |
| "Crew can’t run on {server}" | The server is not x86_64 Linux, or its system libraries are too old. Ask IT for another server. |
| "Biorouter is already setting Crew up on two servers" | Wait for one to finish, then try again. |
| "Crew isn’t installed on {server} yet", after a paste | Choose **Set up and start Crew**, or do [Install Crew yourself](#install-crew-yourself), then try again. |
| "This computer hasn't connected to the server before" | Do [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer), then try again. |
| "The server's identity changed" | Stop and contact IT. |
| "…needs a newer Biorouter background service" | See [Replace an old background service](connections-and-troubleshooting.md#replace-an-old-background-service). |
| "Biorouter read the workspace but not every detail it needs" | From the last output line, copy `socket`, `workspace_id`, `host_uid` and `workspace_public_key` into **Socket path**, **Workspace ID**, **Host user ID** and **Workspace key**. |
| Any other message | Do what it says, or run the commands yourself. |

Every setup code is listed in [Setup refusals](command-line.md#setup-refusals).

### Create the workspace

1. In step 3, note the workspace **Fingerprint**. People you invite may ask you to read it out.
2. If you chose **Private** in step 1, step 3 also says "Creating {workspace} also marks it as a {institution} workspace, for everyone who works there. Agents working in {workspace} can then use only models approved for {institution}. This can’t be undone." To use another institution, go **Back** to step 1 and change **Institution** first.
3. Choose **Create workspace**. Crew calls this computer the workspace's "first admin device": its key is what lets you act as host ([Limits of the host role](#limits-of-the-host-role)).
4. If the server asks for a password or a code, type it in the Sign in window.

With Private, Create marks the workspace with your institution, as `biorouter crew host --institution` does. It also makes the workspace open to every member, and saves a first [workspace context](agents-and-chat-access.md#the-workspace-context), drafted from a fixed check of the server: its system, processors, memory, GPUs, storage, schedulers and software. If one of these does not work, Crew says that the workspace was created but the setting was not made, and the workspace works anyway. Mark it later with **Set institution to {institution}…** ([Mark the workspace with your institution](#mark-the-workspace-with-your-institution)), and make the others on the **General** or **Context** tab of "{workspace} settings".

When it worked, the dialog closes and shows the checklist. It asks about your institution first only when Create did not mark the workspace, as described next.

If Create fails, choose **Create workspace** again, or see [Connections and troubleshooting](connections-and-troubleshooting.md). If you closed the dialog too early, choose **Finish creating…** on the "Finish creating {workspace}" card.

## Install Crew yourself

You need this only when Biorouter cannot install Crew for you, for example when **Set up and start Crew** cannot reach the server. Crew runs as `biorouter-crew`, installed once in each account at exactly `~/.local/bin/biorouter-crew`, without administrator rights. Members follow the same steps with their own login. If you have not used a terminal, ask IT to install it.

The quickest way uses the copy built into your Biorouter:

1. In step 2 of the Host dialog, open **Crew isn’t on {server} yet?**, or on a member's "Crew isn’t set up" screen, **Install it yourself**.
2. Choose **Save Crew’s server program…** and save the file, such as `biorouter-crew-1.92.1-linux-x86_64`, on your computer. Crew shows its version and SHA-256.
3. Copy the commands shown and run them in a terminal on your computer and then on the server: the first copies the file to the server with `scp`, the second checks its SHA-256 with `sha256sum`, and the third installs it and prints its version.

To use a downloaded release instead, the examples below use version `1.91.2` and the login `alice@hpc.example.edu`. Replace both with your own.

1. In Terminal on your Mac, sign in with `ssh alice@hpc.example.edu`, and run `command -v biorouter-crew`. If SSH asks "Are you sure you want to continue connecting", type `no`, and [verify the server](joining-a-workspace.md#verify-the-server-on-this-computer) first.
   - A path that ends in `/.local/bin/biorouter-crew`: Crew is already installed. Type `exit`. A host goes on to [Host the workspace](#host-the-workspace). A member goes on to [Join from the Biorouter app](joining-a-workspace.md#join-from-the-biorouter-app), or chooses **Try again** if Crew already shows "Crew isn’t set up".
   - Another path, such as `/usr/bin/biorouter-crew`: go to step 5.
   - Nothing, only the prompt again on the next line: go to step 2.
2. From the newest release on the [Biorouter release page](https://github.com/BaranziniLab/biorouter/releases), download `biorouter-cli_1.91.2_amd64.deb`.
3. Open a second Terminal window with Command+N. In it, run these lines on your Mac. Keep the colon at the end of the `scp` line.

   ```bash
   shasum -a 256 ~/Downloads/biorouter-cli_1.91.2_amd64.deb
   scp ~/Downloads/biorouter-cli_1.91.2_amd64.deb alice@hpc.example.edu:
   ```

   The value the first line prints must match the `sha256:` value beside the file on the release page. If it does not, download the file again. The `scp` line may ask for your password or code, then prints a progress line that ends at 100%.

   - For a port other than 22, start the `scp` line with `scp -P 2222`, with a capital P and your port.
   - For a jump host, start the `scp` line with `scp -J gateway.example.edu`, with its name.

4. In the window signed in to the server, run this line. It takes the Crew file out of the package without installing it, and prints nothing when it works. If you see `dpkg-deb: command not found`, ask IT for the file.

   ```bash
   dpkg-deb --extract biorouter-cli_1.91.2_amd64.deb staging
   ```

5. In the window signed in to the server, type this line with the file's full path on the server, and press Enter. After step 4, the path is `$HOME/staging/usr/bin/biorouter-crew`, as below. Otherwise, use the path from step 1, such as `/usr/bin/biorouter-crew`.

   ```bash
   VERIFIED_BINARY="$HOME/staging/usr/bin/biorouter-crew"
   ```

6. Copy these three lines, paste them into the same window, and press Enter.

   ```bash
   mkdir -p "$HOME/.local/bin"
   install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"
   "$HOME/.local/bin/biorouter-crew" --version
   ```

When the install worked, the last line prints the version, such as `biorouter-crew 1.91.2`. For other ways to get the file, see [Get a verified file](administration.md#get-a-verified-file).

## Mark the workspace with your institution

Create marks the workspace when you chose Private and an institution in step 1. Otherwise, such as when you finish a setup from **Finish creating…**, or chose Public on a connection that names an institution, the dialog asks after Create "Mark {workspace} as a {institution} workspace?". Until the workspace has an institution, people can chat and share files, but agents cannot work there.

- **Mark as {institution}** sets it permanently. For another institution, host a new workspace.
- **Not now** leaves it for later.

To set it later:

1. Choose **Set institution to {institution}…** in the checklist, in the note above the message box, or in **Privacy…** in the workspace menu.
2. Choose **Set {id} permanently**. Enter cancels.

The **Institution** row in **Privacy…** now shows it instead of "Not set". **Not now** on the note hides the note on this computer.

If Crew says "Add your institution in Connection settings first.":

1. In **Connection settings…**, choose **Private**, and fill in **Institution**.
2. Choose **Save connection**.
3. Set the institution again, from step 1 above. The **Institution** row shows the short ID.

## Get the workspace ready

When the Host dialog closes, the checklist "Get {workspace} ready" appears. Work from the top.

1. In "Your name: Use “{name}”?", choose **Use**, or **Edit…** to change it. Do this before you invite anyone, because an invitation carries your name as it was then. The row appears when your server account has a full name.
2. Choose **Set institution to {institution}…**, as described above.
3. Choose **Create team**, type a **Name** of up to 64 characters, such as `Analysis Lab`, and choose **Create team**. See [Team name rules](teams-channels-and-people.md#team-name-rules).

The team appears with its `#general` channel, which you own, and the checklist closes. From then on, invite people with **Invite people to {workspace}…** in the channel's welcome block or the workspace menu.

## Choose who manages the workspace

In the workspace menu, choose **People…**, **Privacy…** or **Agent access…**, then the **General** tab of "{workspace} settings". Under **Who can manage teams and channels**:

- **Everyone can manage**: "Every member can rename, archive and delete teams and channels, add people to them and join any of them." Members can also invite people and let them in.
- **Owners manage**: "A team’s creator and a channel’s owner change them, and nobody else." Only you invite people and let them in.

The choice applies at once. Either way, only you remove people from the workspace, change its privacy or delete it. Members see the setting as **Who manages**, "Everyone" or "Owners". A workspace created before this choice existed is managed. From a terminal, run `biorouter crew workspace governance open` or `managed`.

In an open workspace, a person who joins is added to every team and every channel that is not archived, a new team gets every member, and a new channel gets every member of its team. Adding people this way does not end agent access. Read [Open workspaces trust every member](privacy-and-security.md#open-workspaces-trust-every-member) before you choose.

## Invite people

Each person's own Biorouter sets Crew up in their server account when they join, so they need nothing installed first. In an open workspace, every member can do what this section and the next describe.

1. Open the workspace menu and choose **Invite people to {workspace}…**.
2. In **Username**, type the person's login on the server, such as `crew_jack`, not their name.
3. Choose **Invite**, and check that the result names the right person.
4. Choose **Copy** under "Send {first} this invitation:". Send the message by email or chat. As the note under it says, also tell the person the server's SSH fingerprint, which starts with `SHA256:`, or where to get it from IT. Each person checks it once ([Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer)). It is not the workspace fingerprint in your workspace menu.
5. Choose **Invite another**, or **Done**. The person appears under "Waiting to join" in the sidebar, marked "invited".

An invitation is for one person and expires after 24 hours. Inviting the person again replaces it. The message holds no secret.

To add another computer for an existing member, turn on **Add another device for @{name}** and choose **Invite**. If you invite a member without it, Crew says "@{name} is already in {workspace}. To add this computer of theirs, Add another device for @{name} is now on: choose Invite again."

If the person still sees "Crew isn’t set up" after joining, they choose **Set up Crew for my account** on that screen. If that cannot work, for example because the server cannot reach their account, send them or IT the lines under **If {first} sees “Crew isn’t set up”**. They run in the person's own account. Then the person chooses **Try again**.

## Let people in

When the person joins, their Biorouter shows a code of 16 letters and numbers, such as `7QK2-M9XA-3JTP-WZ4D`, which they send you. It lets in only their computer.

1. In the sidebar, under "Waiting to join", choose **Let in…** beside their name.
2. Paste the code into **Code from {first}**. Use only a code that came from that person.
3. Choose **Let {first} in**. The dialog shows "Code saved".
4. Wait for "{first} joined {workspace}", which appears when their computer checks in.
5. In an open workspace, the person is already in every team and channel, so choose **Done**. In a managed one, tick channels under **Channels in {team}**, and choose **Add to {team}**. As the dialog says, the person can then read everything already posted in those channels, files included. The dialog shows, for example, "Added Bob Lee (@bob) to {team}. Bob can now see #general and #methods." A person in no team sees no channels.

"Code saved" does not mean the person has joined. The fingerprint in the dialog is not the code. The code field accepts dashes, spaces and lowercase, reads I and L as 1 and O as 0, and refuses U.

A code you or a member enters works only while that person may still let people in. If a member who invited someone is removed, or the workspace becomes managed, their invitation stops working and the joiner is told to ask the host to invite them again.

For another computer of a member, invited with **Add another device for @{name}**, the dialog lets in "{first}’s new computer". Its button reads **Let {first}’s new computer in**, and once that computer checks in, the dialog shows "{first}’s new computer is in {workspace}". The person keeps their teams and channels, so choose **Done**.

If you closed the dialog before step 5 in a managed workspace, choose **Add to a team…** on the person's row under "Joined, not in your teams".

"Waiting to join" lists people by username, and its rows read "invited", "Code entered" or "Invitation expired". Only you see it in a managed workspace, and every member in an open one. With more than five people waiting, it shows the first five and **Show {n} more**. A row that warns of a different code always shows.

After "Invitation expired", choose **Invite again…** on the row, and follow [Invite people](#invite-people) from step 2.

To cancel an invitation:

1. In the workspace menu, choose **People…**.
2. On the person's row under "Waiting to join", choose **Cancel invitation**.
3. At "Cancel @{username}’s invitation?", choose **Cancel invitation**. The row leaves "Waiting to join".

### If a computer showed a different code

"A computer trying to join as @{username} showed a different code" means you mistyped the code, or another computer tried to join as that person.

1. Check the code with the person directly.
2. Choose **Let in…** on their row, or **Enter the code again**.
3. Enter the code they sent, and choose **Replace code**. The dialog shows "Code saved" again. Continue from step 4 of [Let people in](#let-people-in).

Never approve a code you did not get from the person. "@{username} has no pending invitation" means the invitation expired or was cancelled. Invite them again.

## Add people to teams and channels

In an open workspace, every member of a team or channel adds people to it. In a managed workspace, you can add members to teams and channels that someone else owns, if you are in them. For a team or channel you are not in, ask its owner. The person does not need to accept, and must be in a team before joining its channels. To add people to someone else's channel:

1. In the sidebar, point at the team, choose **⋯**, then **Add people to {team}…**.
2. Tick the people, and the channel under **Also add to**.
3. Choose **Add**. The dialog shows "Added {people} to {team}." and the channels they can now see. Choose **Done**.

The team dialog lists only people not in the team yet. To add someone already in the team to a channel you are in but do not own, use a terminal ([Add people to teams and channels](command-line.md#add-people-to-teams-and-channels)):

1. Open a terminal on your own computer.
2. Run `biorouter crew members add @bob --channel '#methods'`. It prints a line such as `Added "Bob Lee" (@bob) to #methods.`

## Manage the workspace

In the workspace menu, **People…**, **Privacy…** and **Agent access…** open the settings dialog, titled with the workspace's name, such as "chen-lab settings", once the connection is verified. Its tabs are **General**, **People**, **Privacy**, **Context** and **Agent access**. Several changes below end every agent's access ([Why settings changes end access](agents-and-chat-access.md#why-settings-changes-end-access)).

### Watch how full the workspace is

The **Storage** row on the **General** tab shows you, and only you, how full the workspace is, such as "64% full". From 80%, the Crew sidebar also shows a **Storage** row such as "chen-lab is 82% full." When it is full, only removing people and privacy changes still work, and a new workspace is the way to keep posting. See [Workspace limits](administration.md#workspace-limits).

### Keep the workspace context

The **Context** tab holds the [workspace context](agents-and-chat-access.md#the-workspace-context), the description of the server that every member's agent reads. In an open workspace every member edits it, and in a managed one only you do.

1. Edit the text, or choose **Draft from server** to draft it again from what the server reports. A draft stays unsaved until you save it.
2. Choose **Save context**. "Context saved." appears, and the line above the editor shows the revision and who saved it.

If someone saved a newer version while you were editing, Crew keeps your text and says so. Save again to replace theirs, or choose **Use their version**. The context can be at most 64 KB.

### Rename the workspace

1. In "{workspace} settings", open **General** and choose **Rename…**.
2. Type the new name in lowercase letters, numbers and dashes, up to 40 characters.
3. Choose **Rename**. The dialog closes, and the new name appears at the top of the sidebar.

A rename keeps history, invitations and agent access. The server folder keeps the original name.

### Remove someone from the workspace

1. In **People…**, point at the person's row, choose the options button (three dots), then **Remove from {workspace}…**.
2. Type their username exactly, with the same capitals.
3. Choose **Remove from {workspace}**. Enter does not confirm. The person's row leaves **Members**.

The person loses the workspace on every computer and sees "You’re no longer in {workspace}". Their messages stay, marked as from a former member. Removing someone from one channel is the channel owner's task, or in an open workspace any member's of that channel.

### Change the workspace privacy

In **Privacy…**, the **Workspace** row reads "Private for everyone", the default, or "Allows Public".

To allow Public:

1. Choose **Allow Public…**.
2. Type the workspace name, and choose **Allow Public**. The **Workspace** row reads "Allows Public".

To make it Private again:

1. Choose **Make Private for everyone…**.
2. Choose **Make Private for everyone**. The **Workspace** row reads "Private for everyone".

See [Privacy and security](privacy-and-security.md#change-the-workspaces-privacy-host-only) for what each setting allows.

### Delete the workspace

Deleting the workspace removes it for everyone, and cannot be undone. Copy anything you need first ([Back up a workspace](administration.md#back-up-a-workspace)).

1. In "{workspace} settings", open **General**. Under "Danger zone", choose **Delete workspace…**.
2. At "Delete {workspace} for everyone?", read what goes, type the workspace's name, and choose **Delete workspace**.

Crew on the server refuses every request from then on, waits 30 seconds so connected members learn of it, then deletes everything the workspace kept there: its teams, channels, messages, files and members, and stops. It deletes only files Crew created, and leaves anything else in that folder alone. The workspace is removed from this computer too, and "Deleted {workspace}." appears. Each member's connection bar says "The host deleted {workspace}." with **Remove from this computer…**, which is all they can do.

**Delete workspace…** is unavailable, with "Update Crew on {server} to delete this workspace.", when the server's Crew is older than deletion. From a terminal, run `biorouter crew workspace delete`, which asks you to type the name. If the server program is stopped, `biorouter-crew delete` on the server does the same ([Delete a stopped workspace](administration.md#delete-a-stopped-workspace)).

**Remove from this computer…** in the same section removes the workspace from this computer only, as [Connection settings](connections-and-troubleshooting.md#connection-settings) describes.

### Limits of the host role

You cannot hand the host role to someone else, or be removed. In a managed workspace you cannot transfer, rename or archive another person's channel. You cannot start, stop or approve another person's agent, act as them, or see their password, codes or device keys.

> **Warning.** Only your own computers can act as host, through their device keys. Never remove the workspace from your only computer, because nothing can restore the host controls. If you try, Crew says "You host {workspace}, and no other computer of yours can act as its host." and asks you to type the workspace's name before **Remove and give up hosting**.

To add a second computer:

1. Invite your own username with **Add another device for @{username}** turned on.
2. [Join](joining-a-workspace.md) on the second computer with that invitation.
3. Let it in from this computer with the code it shows.

The second computer opens the workspace and can act as host too.

## After the server restarts

Crew does not start by itself, and members see "Crew isn’t running on {server}". Your own Biorouter shows the same screen with the start line from step 4, with the workspace's folder filled in, and **Copy**. If you have not used a terminal, ask IT to do this as your account.

1. Open Terminal (press Command and Space, type Terminal, and press Return), and sign in with `ssh` and your server login.
2. Run `ls "$HOME/.local/share/biorouter-crew"`. It lists one folder per workspace, under the workspace's original name, such as `wong-lab`.
3. Type this line with that folder name, and press Enter.

   ```bash
   NAME=wong-lab
   ```

4. Copy this line, paste it, and press Enter. Do not add `--name`.

   ```bash
   "$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/$NAME"
   ```

5. `"state":"running"` in the output means Crew is running. For `"state":"starting"`, wait a few seconds, then run the line again with `status` in place of `start`. `status` also prints `"state":"running"` once Crew answers.
6. Members' Crew tries again by itself every few minutes for an hour, so it reconnects soon after Crew runs. Anyone who sees the workspace offline, you included, can choose **Connect to {workspace}** to connect at once.

The status row under the workspace name reads "Connected" again. Starting again keeps the workspace, its members, keys and history. To stop Crew, see [Stop the broker](administration.md#stop-the-broker).

## Less common situations

### Invite someone whose Biorouter is older

If the person's Biorouter, or Crew on your server, is too old for codes, or Crew asks for an enrollment token:

1. In the Invite dialog, choose **Other ways to invite (older Biorouter)**.
2. Paste the person's join request, and type their user ID, which `id -u {username}` prints on the server.
3. Choose **Create invitation**. The hidden token appears, with "It works once and expires in an hour."
4. Choose **Copy**, and send the token. The button reads "Copied".

The person follows [Join with a token on an older server](joining-a-workspace.md#join-with-a-token-on-an-older-server).

### An older Crew on the server

- In the Let in dialog, **Add to {team}** reads **Invite to {team}**, and the person must accept. **Add to a team…** under "Joined, not in your teams" also sends an invitation. See [Less common situations](teams-channels-and-people.md#less-common-situations).
- **Rename…** for the workspace does not appear.
- A Crew from before open workspaces keeps the workspace managed, and offers no **Context** tab and no deletion. Update it with `biorouter crew setup --replace-existing`, then restart it, as [Upgrade Crew](administration.md#upgrade-crew) describes.

## Related documentation

- [Crew user manual](README.md): every page.
- [Joining a workspace](joining-a-workspace.md): what invited people do.
- [Teams, channels and people](teams-channels-and-people.md): teams, channels and who may manage them.
- [Agents and chat access](agents-and-chat-access.md): agent access levels and the workspace context.
- [Privacy and security](privacy-and-security.md): Private, Public, institutions and open workspaces.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connecting and signing in.
- [Command line](command-line.md#host-a-workspace): a command for each host task.
- [Administration](administration.md): server requirements, storage permissions and limits.
