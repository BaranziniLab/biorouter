# Crew command line

> **What this is.** A task reference for the `biorouter crew` commands: their syntax, what they print and their exit status.
> **Status:** Current. Checked against Biorouter 1.91.2 (`biorouter crew --help`) on 2026-09-25.
> **Audience:** IT staff, lab managers, and lab members who prefer a terminal or want to script Crew. You should know how to open a terminal and run a command.

These commands and the desktop app share one background service on your computer, the Biorouter daemon (`biorouterd`), and its saved connections, so both show the same workspace.

Every command here starts with `biorouter crew`, which short examples leave out. Add `--connection lab` when several connections are saved. The examples use workspace `lab`, host `@alice`, new member `@bob`, team `analysis-lab` and channel `#methods`. Capitals mark placeholders. `--help` lists every option.

## Before you start

You need:

- A Mac or Linux computer. On Windows, commands fail with `Shared Crew daemon IPC is unavailable on this platform`.
- `biorouter` and `biorouterd` in the same folder, and the same Biorouter profile as your desktop app.
- An account on the lab's Linux server, with an SSH key, password or code from your IT team.
- Your own `~/.local/bin/biorouter-crew` in that account. Every connecting account needs one, the host's included. A copy in `/usr/bin` does not count. See [Administration](administration.md).
- The server's SSH host key in your known hosts file. See [Connections and troubleshooting](connections-and-troubleshooting.md).

## The approval secret

Every command except `daemon status` first asks for your Crew approval secret, at a hidden prompt that names the `Crew approval secret`. It proves that a person, not a program or an AI agent, is acting. The desktop app asks for the same secret.

- The secret is 32 to 4096 printable ASCII characters, with no spaces.
- You choose it when the daemon starts, and it stays until the daemon stops. Use the same secret every time.
- Biorouter cannot show or recover it. Keep it in your password manager.
- A wrong secret fails with `Daemon returned 403: Authorize this action ...`, which does not say the secret was wrong.

### If you forget the approval secret

`daemon stop` needs the secret too, so end the daemon another way:

1. Quit the desktop app. Quitting it does not stop the daemon.
2. Run `biorouter crew daemon status` and note the pid in `Biorouter daemon running (pid 4242) for this profile.`
3. Run `kill 4242` with your pid, or restart your computer. `daemon status` then prints `No such file or directory (os error 2)`.
4. Run `daemon start`, or open the desktop app, and choose a new secret.

Your connections, keys and workspaces stay. A running task or transfer may stop, so follow [Recover after a restart](#recover-after-a-restart).

### Supply the secret from a script

Add `--approval-key-stdin` and send the secret as the first line of standard input, for example `print-approval-secret | biorouter crew --approval-key-stdin history methods`. Any further input, such as `send --input -` text or the `credentials unlock` passphrase, follows on the next lines. Never put the secret in an argument, shell history, environment variable or file. `auth` always needs a real terminal.

## Global options

These work before or after the command name.

| Option | Effect |
|---|---|
| `--connection NAME` | Chooses a saved connection by name, SSH target or ID. |
| `--show-ids` | Adds IDs to text output. JSON always has them. |
| `--output-format json` or `stream-json` | Prints JSON, indented or one value per line. |
| `--no-start` | Fails instead of starting a daemon. |
| `--expected-mode private` or `public` | Refuses `send`, `tasks start`, `grants grant` and file transfers when the connection is in the other mode. |
| `--expected-policy-epoch N`, `--expected-workspace-policy-epoch N` | Refuse `tasks start` and `grants grant` when the policy changed. See [Privacy settings](#privacy-settings). |

## Output and exit status

Results go to standard output, and prompts and errors to standard error. `watch`, `join`, `tasks watch` and `files watch` print one JSON value per line in both JSON formats.

| Exit | Meaning |
|---|---|
| `0` | Success, or you stopped one of those four with Ctrl+C. |
| `1` | Failed, refused, or not finished, such as `grants revoke` before the workspace confirms. |
| `2` | Wrong command line. Nothing was sent. |

In JSON, an error has `error`, `request_id`, and `code` or `broker_code`. Scripts should match those codes, not the wording. A refusal from your own daemon starts with its HTTP status, such as `Daemon returned 409:`. When a command needs one of two options, the usage line lists both as required. Give exactly one.

## Name people, teams and channels

The daemon looks names up in your own view of the workspace. An ID works anywhere a name does.

- `@bob` is the person whose server username is exactly `bob`. A display name never selects anyone.
- A team is its name or handle, such as `analysis-lab` or `"Analysis Lab"`. Case, spaces and dashes do not matter.
- A channel is `methods`, `'#methods'` or `analysis-lab/methods`. Quote a leading `#`, or the shell drops the word. Once you are in two teams, write `analysis-lab/general`.
- Files, transfers, tasks, chat sessions and remote references take only IDs. `--show-ids` shows them.

Nothing is guessed. An unknown name says so, and an ambiguous one lists the matches. Nothing is sent, and the command exits with `1`.

## Start, check and stop the daemon

- `daemon status` needs no secret. It prints the pid, or exits with `1` and `No such file or directory (os error 2)`, which only means no daemon is running.
- `daemon start` sets or asks for the secret. Most commands start the daemon for you.
- `daemon stop` stops it for the desktop app too. Closing the app does not stop it.

A message that starts `Restart the shared Biorouter daemon` means the daemon is older than the command, so stop and start it. `This daemon has no human approval authority` means it refuses every command, so end it as in [If you forget the approval secret](#if-you-forget-the-approval-secret). After `Stop accepted, but ... shutdown is unconfirmed`, wait until `daemon status` shows no daemon.

## Keep device keys in an encrypted vault

Each computer has a device key for each workspace, kept in your system keyring by default. `credentials status` then prints `Not set up · keyring`, which is normal. To use a passphrase vault instead, run `credentials init` before you prepare a key or save a connection. Keys already in the keyring do not move.

- The passphrase is 1 to 1024 bytes and must differ from the approval secret.
- `credentials lock` and `credentials unlock` close and open the vault. The `credentials` commands never start a daemon.
- If the vault files go missing, restore them from backup. Biorouter never replaces them with a new vault.

## Join a workspace

Save the invitation your host sent, or only its `brcrew1:` line, in a file such as `lab.txt`. It expires 24 hours after the host invites you.

1. Run `biorouter crew connections join-invitation ./lab.txt --preview`. It saves nothing. `You'll join as` shows how your computer will treat the workspace. Your host can compare the fingerprint.
2. Run it again without `--preview`. Type `y` and press Enter. It prints `Saved lab.`
3. Run `biorouter crew auth` and type your password or code. It prints `Authenticated. The connection is ready.`
4. Run `biorouter crew join`. It prints `Send Alice this code: 7QK2-M9XA-3JTP-WZ4D` and waits.
5. Send the code to your host, for example by Slack or email.
6. When the host enters it, `join` prints `You're in lab.`

To paste the invitation, pass `-`, end with Ctrl+D, and add `--yes`. If saving fails, the message names the option to add. One computer cannot use two institutions on one server.

`--mode`, `--institution`, `--username` and `--name` override the invitation's values. `--ssh-target` uses a login from your SSH settings and ignores the invitation's port and jump host. `--port`, `--identity-file` and `--proxy-jump` set SSH details, and an empty `--proxy-jump` means none. `--remote-root PATH` gives agents a server work folder, for private models only, and `--remote-execution` lets them run commands there. See [Administration](administration.md).

While `join` waits:

| You see | What to do |
|---|---|
| `The code Alice entered doesn't match this computer. ...` | Send the code again. |
| `You're not in lab yet. Ask "Alice Chen" (@alice) to invite your account on the server.` | Ask the host to run `enroll invite @bob`. |
| `This invitation expired. ...` | Ask for a new invitation. |
| `This workspace's server can't let people join with a code yet. ...` | See [Tokens for older servers](#tokens-for-older-servers). |

Ctrl+C stops waiting and keeps the invitation open. `join --no-wait` prints the state and returns, for scripts.

## Sign in and stay connected

- `status` or `connections list` prints one line per connection, such as `lab · bob@hpc.example.edu · Connected · Private (ucsf)`, plus the last error. `connections show` details one.
- `auth` signs in inside your terminal and connects.
- `connect` connects without prompts. If the server wants a password or code, it fails with `crew_ssh_auth_required`, so run `auth`.
- `disconnect` closes the connection until you connect again.

The daemon reconnects by itself after a network drop, but not after `disconnect`, a password prompt, a server key problem or removal. See [Connections and troubleshooting](connections-and-troubleshooting.md). A removed computer shows `Last error: This computer is no longer a member of lab.`

| SSH failure code | What to do |
|---|---|
| `crew_ssh_host_key_unknown` | Get the key fingerprint from IT, check it, and add the key to your known hosts file. |
| `crew_ssh_host_key_changed` | Do not connect. Ask IT to confirm the change. |
| `crew_ssh_unreachable` | Check your network, or your VPN (the app that connects you to your institution's network). The daemon keeps trying. |
| `crew_bridge_missing` | Ask your host or IT to install `~/.local/bin/biorouter-crew` in your account. |
| `crew_workspace_identity_mismatch` | Stop. The server answered for a different workspace. Ask your host. |
| `crew_ssh_failed` | SSH or Crew on the server failed, for example because Crew stopped when the server restarted. Ask your host to start it again as in [Server commands](#server-commands), then run `connect`. |

`auth` says `Signed in, but Crew couldn't start on the server` when that copy is missing, and `terminal could not be attached` when another window is already signing in to this connection.

## Change or remove a saved connection

`connections remove` acts at once, with no confirmation prompt. It disconnects, ends every chat's access through that connection, and deletes this computer's device key for the workspace. Your messages stay on the server, and you stay a member, so your old invitation does not bring the workspace back. To use it here again, follow [Add this computer to your existing account](joining-a-workspace.md#add-this-computer-to-your-existing-account), or from a terminal:

1. Ask the host to run `enroll invite @bob --add-device` and send you the new invitation.
2. Run `connections join-invitation` with it, then `auth` and `join`, as in [Join a workspace](#join-a-workspace).
3. Send the new code to the host. When the host enters it, `join` prints `You're in lab.`

If you are the host, never remove the workspace from your only computer. Host tasks, such as inviting and removing people, work only from your computers, so nobody could do them again. Add a second computer first with `enroll invite @alice --add-device`.

`connections save FILE` adds a connection from a JSON description, and `connections update FILE` replaces the selected one. The fields are listed in [Save a connection from a descriptor](../research/biorouter-crew/cli-guide.md#save-a-connection-from-a-descriptor). The output of `connections show` is not valid input.

## Host a workspace

In the desktop app, choose **Host a new workspace**, then **Start it for me**, as [Hosting a workspace](hosting-a-workspace.md) describes. From a terminal:

1. On your computer, run `biorouter crew connections prepare --show-ids` (`enroll prepare` is the same command). Note the public key and the `Preparation ID:`, which only `--show-ids` shows.
2. On the server, in your own SSH session, run the command below. The name is 1 to 40 lowercase letters, numbers and dashes, and starts and ends with a letter or number. Copy the JSON it prints into a file on your computer, such as `lab-start.txt`.

   ```bash
   "$HOME/.local/bin/biorouter-crew" start --name lab --bootstrap-key 'PUBLIC_KEY_HEX'
   ```

3. On your computer, run `biorouter crew connections join-invitation ./lab-start.txt --preparation-id 'PREPARATION_ID' --ssh-target alice@hpc.example.edu --institution ucsf`.
4. Run `biorouter crew auth`, then `biorouter crew workspace bootstrap`, which works once, with the key you gave `start`. It prints `Signed in to lab as @alice.` Members see your display name after you run `profile set 'Alice Chen'`.
5. Run `biorouter crew privacy set-workspace private --institution ucsf`. The institution can never change, so check it before anyone runs an agent.

`workspace show` lists its privacy, people, teams, channels, invitations and agent grants. `workspace rename NAME` (host) keeps the history and grants.

## Let people into the workspace

Only the host runs `enroll` commands.

1. Run `biorouter crew enroll invite @bob`. Crew checks that this server account exists and prints an invitation message. Send the whole message to Bob. It holds no secret.
2. When Bob sends his code, run `biorouter crew enroll approve @bob 7QK2-M9XA-3JTP-WZ4D`. Case, spaces and dashes do not matter. It prints `Code saved. @bob joins when their computer confirms the same code.`
3. Bob is in only when his `join` prints `You're in lab.`

`enroll pending` lists who is waiting. If you mistyped a code, Bob's `join` reports a mismatch and `enroll pending` warns under his row. Run `enroll approve` again with `--replace`. Approve only a code the person sent you.

- `connections invitation --for @bob` prints the invitation again. `enroll invite @bob --add-device` lets a member add another computer. `enroll cancel @bob` withdraws an invitation.
- An invitation lasts 24 hours. Inviting again replaces it. At most 100 people can wait at once.
- `enroll revoke @bob` removes a member with all their computers, and asks you to type `@bob` again. In a script, add `--confirm @bob`. It ends every agent grant in the workspace. A removed person invited again starts with no teams or channels.

### Tokens for older servers

A server whose `biorouter-crew` cannot join by code needs the older token path. Its commands (`enroll invite --uid UID --public-key HEX`, then `enroll accept`) are hidden from `--help`, and a token works once, within one hour. See [Enroll with a token](../research/biorouter-crew/cli-guide.md#enroll-with-a-token-older-versions), and update the server when you can.

## Manage your profile, teams and channels

| Command | Notes |
|---|---|
| `profile show` | Your name and enrolled computers. |
| `profile set 'Bob Lee' --avatar 'BL'` | Up to 12 characters of initials or emoji. Leaving out `--avatar` removes them. |
| `teams list`, `teams create NAME` | A new team gets a `general` channel. |
| `teams rename TEAM NAME` | Team creator only. |
| `channels list [--team TEAM]` | Shows classification, owner and unread count. |
| `channels create NAME --team TEAM` | `--classification restricted` (default) or `public-safe` (JSON writes `public_safe`). |
| `channels rename`, `channels archive` | Owner only. An archived channel stays readable and takes no posts. |
| `channels mark-read CHANNEL` | Marks the channel read. |

A display name is a label only. It cannot contain `@` or `#` or match a username. Only private AI models can read a Restricted channel. See [Privacy and security](privacy-and-security.md). Name rules are in [Teams, channels and people](teams-channels-and-people.md). After ten names that were already taken within ten minutes, you get `Too many name attempts. Try again later.` Wait ten minutes.

## Add people to teams and channels

- `members` lists everyone, marking `you`, `host`, `former member` and `account no longer valid`.
- `members add @bob --team TEAM` (team owner or host) adds Bob to the team and its `#general`, and to each `--channel` of that team. Bob does not need to accept.
- Without `--team`, each `--channel` must be one you own, in a team Bob is already in. The host may name any channel there.

| Task | Command |
|---|---|
| Invite someone, who then accepts (team creator or channel owner) | `invites create @bob --team TEAM` or `--channel CHANNEL` |
| Accept an invitation | `invites list`, then `invites accept [TEAM or TEAM/CHANNEL]` |
| Remove someone from a channel (channel owner) | `remove-member CHANNEL @bob` |
| Hand over a channel (owner, then Carol) | `ownership offer CHANNEL @carol`, then `ownership accept CHANNEL` |

A channel invitation needs the person in its team first. `invites accept` exits with `1` when nothing is pending. Add `--former` to `remove-member` for someone who left the workspace. After a handover, the previous owner leaves the channel.

## Post and read messages

- `send methods --text 'Ready.'` posts under your name. `--input FILE` reads the text from a file, or `-` for standard input. `--attachment ID` and `--reference ID` add files and can repeat.
- `history methods --latest` shows the newest 100 messages. Without `--latest`, it starts from the oldest. `--limit` takes 1 to 200. With `--show-ids`, it ends with a `Cursor:` line to pass to `--before` or `--after`.
- `search methods 'qPCR'` finds messages in one channel, in any case.
- `watch methods` prints messages as they arrive. Ctrl+C stops it without cancelling tasks. If the daemon ends it, for example because you lost access, it prints why and exits with `1`.

Crew sends no system notifications, so keep a `watch` running to follow a channel. Message limits are in [Messages and files](messages-and-files.md).

## Share files and server paths

Uploading does not post. You upload, then attach:

1. `biorouter crew --show-ids files upload methods ./counts.csv` prints the transfer ID.
2. `files watch TRANSFER_ID` follows it to `Transfer Ready.`
3. `--show-ids files status TRANSFER_ID` shows the `Attachment ID:`.
4. `send methods --attachment ATTACHMENT_ID` posts it.

To download, get the ID from `history methods --latest --show-ids`, then run `files download ID --output ./counts.csv`. Add `--overwrite` to replace a file.

- `files pending` lists transfers. `files pause ID` pauses one, and `files resume ID FILE` resumes it with the original file.
- `files forget ID` removes a record. For an unfinished download, add `--file PATH` to delete the partial file.
- `files watch` exits with `0` however the transfer ends, so read the last state: `Ready`, `Saved`, `Paused`, `Failed` or `Not confirmed`. Ctrl+C leaves the transfer running.

Crew refuses to save into a folder that is not yours or that other accounts can change, or into a credential or settings folder. It refuses to upload files that look like credential stores. Size limits are in [Administration](administration.md).

To share a server path without uploading it, run `--show-ids files reference methods /project/results --label 'Results'`, then `send methods --reference ID`. Crew does not check the path. `files show-reference ID` shows one.

## Run an agent task

A task runs your own AI agent on a prompt, lets it read the channel, and posts its result there.

```bash
biorouter crew tasks start methods --input ./prompt.txt \
  --provider PROVIDER --model MODEL --allow-posting
```

- Give the prompt with `--text` or `--input`. `--allow-posting` is required.
- `--provider` is the name in your Biorouter configuration.
- Each `--context-channel CHANNEL` adds a channel to read, up to 16.
- The task's access lasts one hour.
- In a Private workspace, the model must be approved for the workspace's institution, or run locally. See [Agents and chat access](agents-and-chat-access.md).
- The result ends with a line naming the shared files it read, if any.

`tasks list --show-ids` shows each task and its chat session, and `tasks show ID` shows one. `tasks watch ID` follows one and exits with `0` at every end state, so read the last: `Done`, `Couldn't finish`, `Stopped`, `Interrupted`, `Outcome unknown` or `Stop not confirmed`. Ctrl+C stops watching, not the task.

`tasks cancel ID` stops the task on your computer and revokes its access, but cannot confirm that a process on the server ended. If the workspace cannot be reached, it exits with `1` and gives a `--request-id` to retry with.

## Give a chat access to Crew

The desktop app does this with `/crew`, as [Agents and chat access](agents-and-chat-access.md) explains. From a terminal, find the chat's session ID with `biorouter session list`. The chat must be open in the shared daemon, in the desktop app or a terminal chat, and idle.

```bash
biorouter crew grants grant SESSION_ID methods --context-channel analysis-lab/raw-data
```

The first channel is where the chat posts. Each `--context-channel` adds one to read, up to 20 channels in all. `grants list` shows each chat and task with access, its state and time left. `context SESSION_ID` shows one grant's channels.

These changes end every grant and task in the workspace. `grants list` then shows `Ended: Crew settings changed`, and you grant access again:

- adding or removing people, or accepting an invitation
- archiving a channel, or offering or accepting its ownership
- changing the workspace's privacy

`grants revoke SESSION_ID` exits with `0` only when the workspace confirms. After `Stopped on this device ...` (exit `1`), the chat already cannot use Crew, and Biorouter confirms later by itself. `Not revoked` means the chat still has access, so retry. A revoke does not prove that a command already running on the server stopped.

To use a terminal chat, create it with `biorouter session --shared-daemon --no-start --create-only`, which prints its session ID. Grant it, then open it with `biorouter session --shared-daemon --no-start --resume --session-id SESSION_ID`, or send one prompt with `biorouter run` and the same options plus `--text`. Without `--shared-daemon`, the chat cannot use a grant.

## Privacy settings

`privacy show` prints the privacy, institution and policy epoch of your connection and of the workspace, and each channel's classification.

- `privacy set-personal private --institution ID` or `public` changes how this computer treats the workspace.
- `privacy set-workspace private` or `public` (host) changes the workspace's privacy. `--institution ID` confirms its institution, which never changes.

An institution ID is 1 to 64 lowercase letters, digits, `_` or `-`. Changing the workspace's privacy ends every agent grant. Switching to Public never exposes earlier Private content. See [Privacy and security](privacy-and-security.md).

To stop a script when the policy changed after it checked, pass the epochs from `privacy show` to `--expected-policy-epoch` and `--expected-workspace-policy-epoch`. For posts and file transfers, use `--expected-mode private`.

## Retry after an uncertain result

If a change may have reached the workspace but the answer was lost, the error ends with `Retry safely with --request-id ID`. Run the same command again with that option. The workspace does not apply the change twice. A clear refusal never shows this line.

An ID is 1 to 128 letters, digits, `_` or `-`. Use a new ID for a different change. To continue a transfer, use `files resume`, not a new upload.

## Recover after a restart

After your computer or the daemon restarts, check in this order:

1. Run `daemon status`. Any command starts a missing daemon.
2. Run `credentials unlock` if you use the vault.
3. Run `status`. Run `auth` or `connect` for a `Disconnected` connection.
4. Run `files pending`, `tasks list` and `grants list`.
5. For a task in `Interrupted`, `Outcome unknown` or `Stop not confirmed`, read its channel before you start it again. Repeat `tasks cancel` if the message asks.

If a command reports a changed key or a daemon it cannot verify, find out why before you connect. Do not replace keys or files to silence it.

## Workspace refusals

Workspace refusals are plain sentences that name the fix, with the code in JSON as `broker_code`. Three need a note:

- `The workspace didn't allow this.` You lack the role, such as host or owner.
- `Account enrollment changed.` Your server account was renamed or replaced. Ask the host to remove you and invite you again.
- `The workspace refused this request.` Run the command with `--output-format json` and send the `broker_code` to your host.

## Server commands

The server program `biorouter-crew` runs in the host's account. Members never run it, because their daemons start it over SSH. Its commands are in [Administration](administration.md). `status --name` and `stop --name` take the workspace's original name.

Nothing starts the workspace when the server boots. After a reboot, the host runs this, without `--name`:

```bash
"$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/<original name>"
```

The state folder keeps its original name after a rename, and adding `--name` then fails with `name_mismatch` or a `bootstrap_key` error.

## Related documentation

- [Crew user manual](README.md): where each topic lives.
- [Getting started](getting-started.md): the words this manual uses.
- [Hosting a workspace](hosting-a-workspace.md): hosting and restarting a workspace.
- [Joining a workspace](joining-a-workspace.md): joining from the app.
- [Teams, channels and people](teams-channels-and-people.md): owners and name rules.
- [Messages and files](messages-and-files.md): messages, files and their limits.
- [Agents and chat access](agents-and-chat-access.md): tasks, model rules and `/crew`.
- [Privacy and security](privacy-and-security.md): privacy, institutions and classifications.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connection states and reconnection.
- [Administration](administration.md): server setup, limits, upgrades and backups.
- [Crew CLI guide for developers](../research/biorouter-crew/cli-guide.md): design notes and description fields.
