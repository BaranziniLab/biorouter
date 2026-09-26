# Hosting a workspace

> **What this is.** The host's tasks as numbered steps: starting Crew on the lab server, creating the workspace, letting people in, and managing it afterwards.
> **Status:** Current. Checked against the Crew code on 2026-09-25.
> **Audience:** The lab member who hosts a Crew workspace, including people who have never used SSH or a terminal.

A Crew workspace runs in the host's account on a Linux server. Braces mark a name Crew fills in, such as {workspace}, {server}, or {first}, the first name of the person you invite.

## What only the host can do

Crew labels you "Hosted by {name}" in the workspace menu and "Host" in the member list. Only you can invite people, let them in, cancel invitations, remove people, rename the workspace, change its privacy and institution, and add people to teams and channels someone else owns. The server accepts these actions only from a person at your computer, never from an agent.

The workspace runs on the server, so people can use it while your computer is off. After every server restart, you start Crew again ([After the server restarts](#after-the-server-restarts)).

The workspace is stored in your account, in `~/.local/share/biorouter-crew/` under its original name. Your account and the server's administrators can read all of it ([What other people can see](privacy-and-security.md#what-other-people-can-see)).

## Before you start

| You need | Details |
|---|---|
| Biorouter on your computer | The first time it opens, you choose an [approval secret](getting-started.md#the-approval-secret). |
| A login on a Linux server Crew can run on | Ask IT whether the server is 64 bit Intel or AMD Linux (x86_64) with glibc 2.31 or newer, whether your home folder is on local disk and writable only by you, and whether its name always reaches the same machine. If not, use one machine's name. See [Check a server before you host](administration.md#check-a-server-before-you-host). |
| SSH access | For **Start it for me**, the server must accept this computer's SSH key, a key pair IT often sets up, without a password or a code. |
| The server's fingerprint | A value that starts with `SHA256:`, from IT. |
| Crew in your own server account | See [Install Crew on the server](#install-crew-on-the-server). Each member installs their own copy, or IT does. |
| Your institution's short ID, such as `ucsf` | Agents need it in a Private workspace. A model shown as "Private · UCSF" means `ucsf`. Otherwise, ask IT. |

### Check which way you will start Crew

1. Follow [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer) once, with your own login. This computer then trusts the server.
2. If `ssh` signed you in without a password or a code, use **Start it for me**. Otherwise, use **Run it yourself in a terminal**.

## Install Crew on the server

Crew runs as `biorouter-crew`, installed once in each account at `~/.local/bin/biorouter-crew`, without administrator rights. Members follow the same steps with their own login ([Install Crew in your server account](joining-a-workspace.md#install-crew-in-your-server-account)). If you have not used a terminal, ask IT to install it.

The examples use version `1.91.2` and the login `alice@hpc.example.edu`. Replace both with your own.

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

## Host the workspace

On the first Crew screen, choose **Host a new workspace**. If you already have a workspace, choose its name at the top of the sidebar, then **Add a workspace**, then **Host a new workspace…**. The dialog has three steps: Name, Start and Create.

### Name the workspace

1. In **Workspace name**, type a name, such as `Wong Lab`. Crew shows it as "Your workspace: wong-lab". Anyone who can sign in to the server can see it, so leave out patient and sample IDs.
2. In **Your server login**, type your login, such as `alice@hpc.example.edu`, or the server's short name from your SSH settings.
3. Under **Privacy**, keep **Private** or choose **Public**. With Private, type your short ID in **Institution** if it is empty.
4. Optional: fill in **Your agent on {server}** ([Agents and chat access](agents-and-chat-access.md)), and **Advanced** if IT gave you a port, an identity file or a jump host.
5. Choose **Continue**. Biorouter makes a hosting key, so only this computer can create the workspace. The dialog moves to step 2, Start.

The Privacy choice sets only your own connection. Every new workspace starts "Private for everyone".

### Start Crew on the server

Step 2 shows four commands that make a private folder, start Crew and print its details. The key in them is public. To change the name, choose **Back**.

1. Choose **Start it for me**. The box "What the server printed" shows the output.
2. When Crew has started, the dialog moves to step 3 by itself.

Biorouter runs exactly the commands shown, as you, and never answers a password or code prompt. **Stop** ends a run. Closing the dialog does not stop the commands on the server.

#### Run the commands yourself

Use this when **Start it for me** cannot sign in, or fails.

1. Choose **Run it yourself in a terminal**, then **Open a terminal here**, or use your own terminal.
2. Paste the `ssh` command shown there into the terminal, and press Enter. Check the fingerprint if asked, and type your password or code.
3. Choose **Copy** beside the start commands, paste them into the terminal, and press Enter.
4. Copy all the output, including the whole line from `{` to `}`, into **Paste what it printed**.
5. When Crew shows "Found {workspace} on {server}", choose **Continue**.

#### If starting does not work

A warning appears, and **Run it yourself in a terminal** opens.

| Message starts with | What to do |
|---|---|
| "Crew isn’t installed on {server} yet" | Do [Install Crew on the server](#install-crew-on-the-server), then try again. |
| "Crew on the server said:" | Send it to IT, with [Storage requirements](administration.md#storage-requirements) for `unsafe_storage` or `unsupported_storage`, or [Machine identity](administration.md#machine-identity) for `node_identity_unavailable`. |
| "This computer hasn't connected to the server before" | Do [Check which way you will start Crew](#check-which-way-you-will-start-crew), then try again. |
| "The server's identity changed" | Stop and contact IT. |
| "…needs a newer Biorouter background service" | See [Replace an old background service](connections-and-troubleshooting.md#replace-an-old-background-service). |
| "Biorouter read the workspace but not every detail it needs" | From the last output line, copy `socket`, `workspace_id`, `host_uid` and `workspace_public_key` into **Socket path**, **Workspace ID**, **Host user ID** and **Workspace key**. |
| Any other message | Do what it says, or run the commands yourself. |

### Create the workspace

1. In step 3, note the workspace **Fingerprint**. People you invite may ask you to read it out.
2. Choose **Create workspace**. Crew calls this computer the workspace's "first admin device": its key is what lets you act as host ([Limits of the host role](#limits-of-the-host-role)).
3. If the server asks for a password or a code, type it in the Sign in window.

When it worked, the dialog asks about your institution, as described next, or closes and shows the checklist.

If Create fails, choose **Create workspace** again, or see [Connections and troubleshooting](connections-and-troubleshooting.md). If you closed the dialog too early, choose **Finish creating…** on the "Finish creating {workspace}" card.

## Mark the workspace with your institution

After Create, if Crew knows your institution, the dialog asks "Mark {workspace} as a {institution} workspace?". Until the workspace has one, people can chat and share files, but agents cannot work there.

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

## Invite people

Each person first needs Crew in their own server account ([Install Crew in your server account](joining-a-workspace.md#install-crew-in-your-server-account)).

1. Open the workspace menu and choose **Invite people to {workspace}…**.
2. In **Username**, type the person's login on the server, such as `crew_jack`, not their name.
3. Choose **Invite**, and check that the result names the right person.
4. Choose **Copy** under "Send {first} this invitation:". Send the message by email or chat, with the server's `SHA256:` fingerprint, which each person checks once ([Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer)).
5. Choose **Invite another**, or **Done**. The person appears under "Waiting to join" in the sidebar, marked "invited".

An invitation is for one person and expires after 24 hours. Inviting the person again replaces it. The message holds no secret.

To add another computer for an existing member, turn on **Add another device for @{name}** and choose **Invite**.

If the person sees "Crew isn’t set up" and the server already has a copy in `/usr/bin`, which does not count, send them or IT the lines under **If {first} sees “Crew isn’t set up”**. They run in the person's own account. Then the person chooses **Try again**.

## Let people in

When the person joins, their Biorouter shows a code of 16 letters and numbers, such as `7QK2-M9XA-3JTP-WZ4D`, which they send you. It lets in only their computer.

1. In the sidebar, under "Waiting to join", choose **Let in…** beside their name.
2. Paste the code into **Code from {first}**. Use only a code that came from that person.
3. Choose **Let {first} in**. The dialog shows "Code saved".
4. Wait for "{first} joined {workspace}", which appears when their computer checks in.
5. Under **Channels in {team}**, tick channels, and choose **Add to {team}**. The dialog shows "Added {first} to {team}." A person in no team sees no channels.

"Code saved" does not mean the person has joined. The fingerprint in the dialog is not the code. The code field accepts dashes, spaces and lowercase, reads I and L as 1 and O as 0, and refuses U.

If you closed the dialog before step 5, choose **Add to a team…** on the person's row under "Joined, not in your teams".

Only you see "Waiting to join". Its rows read "invited", "Code entered" or "Invitation expired".

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

You can add members to teams and channels someone else owns. The person does not need to accept, and must be in a team before joining its channels. To add people to someone else's channel:

1. In the sidebar, point at the team, choose **⋯**, then **Add people to {team}…**.
2. Tick the people, and the channel under **Also add to**.
3. Choose **Add**. The dialog shows "Added {people} to {team}." and the channels they can now see. Choose **Done**.

The team dialog lists only people not in the team yet. To add someone already in the team to a channel you do not own, use a terminal ([Add people to teams and channels](command-line.md#add-people-to-teams-and-channels)):

1. Open a terminal on your own computer.
2. Run `biorouter crew members add @bob --channel '#methods'`.
3. Type your approval secret. It prints "Added. @bob can now see #methods."

## Manage the workspace

In the workspace menu, **People…**, **Privacy…** and **Agent access…** open Workspace settings once the connection is verified. Several changes below end every agent's access ([Why settings changes end access](agents-and-chat-access.md#why-settings-changes-end-access)).

### Rename the workspace

1. In Workspace settings, open **General** and choose **Rename…**.
2. Type the new name in lowercase letters, numbers and dashes, up to 40 characters.
3. Choose **Rename**. The dialog closes, and the new name appears at the top of the sidebar.

A rename keeps history, invitations and agent access. The server folder keeps the original name.

### Remove someone from the workspace

1. In **People…**, point at the person's row, choose the options button (three dots), then **Remove from {workspace}…**.
2. Type their username exactly, with the same capitals.
3. Choose **Remove from {workspace}**. Enter does not confirm. The person's row leaves **Members**.

The person loses the workspace on every computer and sees "You’re no longer in {workspace}". Their messages stay, marked as from a former member. Removing someone from one channel is the channel owner's task.

### Change the workspace privacy

In **Privacy…**, the **Workspace** row reads "Private for everyone", the default, or "Allows Public".

To allow Public:

1. Choose **Allow Public…**.
2. Type the workspace name, and choose **Allow Public**. The **Workspace** row reads "Allows Public".

To make it Private again:

1. Choose **Make Private for everyone…**.
2. Choose **Make Private for everyone**. The **Workspace** row reads "Private for everyone".

See [Privacy and security](privacy-and-security.md#change-the-workspaces-privacy-host-only) for what each setting allows.

### Limits of the host role

You cannot hand the host role to someone else, or be removed. You cannot transfer, rename or archive another person's channel. You cannot start, stop or approve another person's agent, act as them, or see their password, codes or device keys.

> **Warning.** Only your own computers can act as host, through their device keys. Never remove the workspace from your only computer, because nothing can restore the host controls.

To add a second computer:

1. Invite your own username with **Add another device for @{username}** turned on.
2. [Join](joining-a-workspace.md) on the second computer with that invitation.
3. Let it in from this computer with the code it shows.

The second computer opens the workspace and can act as host too.

## After the server restarts

Crew does not start by itself, and members see "{workspace} is offline". If you have not used a terminal, ask IT to do this as your account.

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

5. `"state":"running"` in the output means Crew is running. For `"state":"starting"`, wait a few seconds, then run the line again with `status` in place of `start`.
6. Tell members to choose **Connect to {workspace}**, and do the same in your own Biorouter.

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

## Related documentation

- [Crew user manual](README.md): every page.
- [Joining a workspace](joining-a-workspace.md): what invited people do.
- [Teams, channels and people](teams-channels-and-people.md): teams and channels.
- [Privacy and security](privacy-and-security.md): Private, Public and institutions.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connecting and signing in.
- [Command line](command-line.md#host-a-workspace): a command for each host task.
- [Administration](administration.md): server requirements and limits.
