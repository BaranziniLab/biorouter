# Crew command line

> **What this is.** A reference for the `biorouter crew` commands, arranged by task. It gives the syntax of each command, examples, what each command prints and its exit status.
> **Status:** Current. Checked against Biorouter 1.91.2 (`biorouter crew --help`) and the Crew source on branch `codex/biorouter-crew` on 2026-09-25.
> **Audience:** IT staff, lab managers, and lab members who prefer a terminal or want to script Crew. You should know how to open a terminal and run a command.

Crew is the part of Biorouter where a lab chats, shares files and runs AI agents in a shared workspace. The workspace runs on a Linux server under the host's account, and each member reaches it over their own SSH login. The Biorouter desktop app and the `biorouter crew` commands are two ways to use the same workspace. Both talk to one background service on your computer, the Biorouter daemon (`biorouterd`), and both use the same saved connections. A message you post from the terminal appears in the desktop app, and the reverse. Most lab members use the desktop app. This page is for terminal users, for scripts, and for the people who support them.

In the examples, the workspace is `lab`, the host is `@alice`, a new member is `@bob`, the server is `hpc.example.edu`, the team is "Analysis Lab" (handle `analysis-lab`) and the channel is `#methods`. Words in capitals, such as `TRANSFER_ID`, are placeholders you replace.

## Before you start

You need all of the following:

| Requirement | Details |
|---|---|
| A Mac or Linux computer | The commands reach the daemon over a Unix socket. On Windows a command asks for the approval secret and then fails with `Shared Crew daemon IPC is unavailable on this platform`. |
| `biorouter` and `biorouterd` in the same folder | Starting the daemon from the command line needs both. Otherwise it fails with `biorouterd must be installed alongside biorouter for shared Crew startup`. |
| The same Biorouter profile as your desktop app | The desktop app and the command line share one daemon only when they use the same profile. |
| An account on the lab's Linux server, and a way to sign in to it | An SSH key, a password, a verification code, or a mix. Your IT team provides this. |
| `~/.local/bin/biorouter-crew` in your server account | Every account that connects needs its own copy at exactly that path, the host's account included. A copy only in `/usr/bin` or on your `PATH` does not count. See [Administration](administration.md). |
| The server's SSH host key in your known hosts file | Crew never accepts an unknown server key. See [Connections and troubleshooting](connections-and-troubleshooting.md). |

To see the help, run `biorouter crew --help`, or add `--help` after any command, for example `biorouter crew enroll approve --help`. Several commands print no description line in their help. This page gives the meaning of each one.

## Commands at a glance

"Host" in the last column means only the workspace host can run the command. "Owner" means the owner of the team or channel. The host counts as an owner only where the row says "Owner or host".

| Command | What it does | Who |
|---|---|---|
| `daemon start`, `daemon status`, `daemon stop` | Start, check or stop the shared daemon on this computer. | Anyone |
| `credentials status`, `init`, `unlock`, `lock` | Manage the optional encrypted vault for this computer's device keys. | Anyone |
| `status`, `connections list` | List saved connections and their state. | Anyone |
| `connections show` | Show one saved connection in detail. | Anyone |
| `connections join-invitation` | Save a connection from the invitation your host sent. | Anyone |
| `connections prepare` (also `enroll prepare`) | Create this computer's device key for hosting or for the older token path. | Anyone |
| `connections save`, `connections update` | Save or replace a connection from a JSON description. | Anyone |
| `connections remove` | Delete a saved connection from this computer. | Anyone |
| `connections invitation` | Print the invitation message again. | Host |
| `auth` | Sign in to the server over SSH in the terminal. | Anyone |
| `connect`, `disconnect` | Open or close the connection to the workspace. | Anyone |
| `join` | Show this computer's code and wait until the host lets you in. | Joiner |
| `workspace show` | Show the workspace, its people, teams, channels and grants. | Member |
| `workspace bootstrap` | Claim a new workspace with this computer's prepared key. | Host, once |
| `workspace rename` | Rename the workspace. | Host |
| `enroll invite`, `pending`, `approve`, `cancel`, `revoke` | Invite people to the workspace, let them in, withdraw an invitation, or remove a member. | Host |
| `members` | List the people in the workspace. | Member |
| `members add` | Add a member to a team or channel. | Owner or host |
| `remove-member` | Remove a member from a channel you own. | Owner |
| `profile show`, `profile set` | Show or change your display name and avatar. | Member |
| `teams list`, `create`, `rename` | List, create or rename teams. | Member (rename: team creator) |
| `channels list`, `create`, `rename`, `archive`, `mark-read` | List, create, rename, archive or mark channels read. | Member (rename and archive: owner) |
| `invites list`, `create`, `accept` | Invite a member to a team or channel, and accept invitations. | Member (create: team creator or channel owner) |
| `ownership offer`, `ownership accept` | Hand a channel to another member. | Owner, then the new owner |
| `send`, `history`, `search`, `watch` | Post, read, search and follow channel messages. | Member |
| `files ...` | Upload, download and share files and remote references. | Member |
| `tasks start`, `list`, `show`, `watch`, `cancel` | Run your own agent task in a channel. | Member |
| `grants grant`, `list`, `revoke` and `context` | Give a Biorouter chat access to Crew, check it, and end it. | Member |
| `privacy show`, `set-personal` | Show privacy settings, and change how this computer treats the workspace. | Member |
| `privacy set-workspace` | Change the workspace's privacy and confirm its institution. | Host |

## The approval secret

Every `biorouter crew` command except `daemon status` asks for your Crew approval secret before it does anything. The secret proves that a person, not a program or an AI agent, is acting. The desktop app asks for the same secret when it starts the daemon or connects to one that is running.

What you type is hidden, so nothing appears on the screen. Which prompt you see depends on the situation:

| Situation | Prompt |
|---|---|
| No daemon is running, and the command starts one | `Crew approval secret (held separately from your profile):` |
| A daemon is already running | `Crew approval secret (printable ASCII, no spaces):` |
| `daemon start` | `Create/use your separately held Crew approval secret (at least 32 bytes):` |

Rules for the secret:

- It is 32 to 4096 printable ASCII characters, with no spaces.
- You choose it when the daemon starts. Every command needs that same secret until the daemon stops. Use the same secret each time you start the daemon, so you only have one to remember.
- Biorouter cannot show it to you or recover it. Keep it in your password manager.

If you type a different secret than the one the daemon started with, the command fails with this message. It does not say that the secret was wrong:

```text
Error: Daemon returned 403: Authorize this action in the Crew panel or native Crew CLI with your human approval secret. Agent tools use their separate task grant.
```

### If you forget the approval secret

Biorouter cannot show or recover the secret. The running daemon keeps the secret it started with until it stops, and `daemon stop` needs that secret too. To choose a new secret, end the daemon another way:

1. Run `biorouter crew daemon status`. It needs no secret. Note the number after `pid` in `Biorouter daemon running (pid 4242) for this profile.`
2. Quit the Biorouter desktop app if it is open. Quitting it does not stop the daemon.
3. End the daemon process with `kill 4242`, using your number. Restarting your computer or logging out also ends it.
4. Run `biorouter crew daemon status` again. When it prints `Error: No such file or directory (os error 2)`, no daemon is running.
5. Run `biorouter crew daemon start` and choose a new secret. If you open the desktop app instead, it asks for the new secret in a window titled "Set approval secret for shared BioRouter daemon", then asks you to type it again in "Confirm shared daemon approval secret".

Your saved connections, device keys and workspaces stay as they were. Ending the daemon can interrupt a task or file transfer that was running, so follow [Recover after a restart](#recover-after-a-restart) next.

### Supply the secret from a script

A script has no terminal to type into. Without one, a command fails with `An interactive no-echo prompt is required; select --approval-key-stdin explicitly to use a secret input pipe`.

Add `--approval-key-stdin` and send the secret as the first line of standard input. In this example, `print-approval-secret` stands for whatever command prints your secret, such as your password manager's command line tool:

```bash
print-approval-secret | biorouter crew --approval-key-stdin --connection lab history methods --latest
```

- Only the first line is read as the secret. For `send --input -`, `connections save -` and `connections join-invitation -`, the rest of standard input is the message, the JSON or the invitation.
- For `credentials init` and `credentials unlock`, the second line is the vault passphrase.
- Do not put the secret in a command argument, your shell history, an environment variable or a connection file.
- `auth` always needs a real terminal, so it cannot read the secret this way.

## Global options

These options work with every `biorouter crew` command. You can put them before or after the command name, so `biorouter crew members --show-ids --connection lab` works.

| Option | What it does |
|---|---|
| `--connection NAME` | Chooses a saved connection by its name (any letter case), its SSH target or its ID. Required when more than one connection is saved. |
| `--show-ids` | Adds machine IDs (people, teams, channels, tasks, files, transfers) to text output. JSON output always has them. |
| `--output-format text`, `json` or `stream-json` | `text` is the default and is written for people. `json` prints the daemon's answer, indented. `stream-json` prints each value on one line. |
| `--no-start` | Fails instead of starting the daemon when none is running. |
| `--approval-key-stdin` | Reads the approval secret from the first line of standard input instead of a hidden prompt. |
| `--request-id ID` | Reuses an ID when you retry the same change after an uncertain result. See [Retry after an uncertain result](#retry-after-an-uncertain-result). |
| `--expected-mode private` or `public` | Refuses `send`, `tasks start`, `grants grant` and file uploads, downloads and resumes when the saved connection is not in this privacy mode. It never changes the mode. |
| `--expected-policy-epoch N` | Refuses `tasks start` and `grants grant` unless the connection's policy epoch is still `N`. |
| `--expected-workspace-policy-epoch N` | Refuses `tasks start` and `grants grant` unless the workspace's policy epoch is still `N`. |

When you leave out `--connection`:

- With exactly one saved connection, the command uses it.
- With none, it fails: `No Crew connection is saved on this computer. Save the invitation your host sent with biorouter crew connections join-invitation -`
- With several, it fails: `Several Crew connections are saved; choose one with --connection NAME. Run biorouter crew connections list to see them.`

## Output, errors and exit status

### Output formats

Text output names people as `"Bob Lee" (@bob)`, or `@bob` alone when the display name is the same as the username. Channels appear as `#methods`. Machine IDs stay hidden unless you add `--show-ids`. With `--show-ids`, an ID appears in square brackets after the line, such as `[transfer ID ...]`, or on its own indented line, such as `  Preparation ID: ...`.

Results go to standard output. Prompts, questions and error messages go to standard error. Commands that follow something over time (`watch`, `join`, `tasks watch` and `files watch`) print one JSON value per line in both `json` and `stream-json` modes.

Text output escapes terminal control characters, so text that a colleague wrote cannot change your terminal. JSON output escapes them as `\uXXXX`.

### Exit status

| Status | Meaning |
|---|---|
| `0` | The command succeeded. Also `0` when you stop `watch`, `join`, `tasks watch` or `files watch` with Ctrl+C. |
| `1` | The command failed or was refused. The reason is on standard error. |
| `2` | The command line itself was wrong: a missing argument, a value that is not allowed, or two options that cannot be used together. Nothing was sent. Running `biorouter crew` with no command prints the help and exits with `2`. |

Some commands exit with `1` when the work is not finished, even though nothing broke. Each section below says so where it applies. For example, `grants revoke` exits with `1` until the workspace confirms the revocation.

### What an error looks like

In text mode, a failure prints one line (sometimes more) on standard error, starting with `Error:`:

```text
Error: No live shared daemon is available; run biorouter crew daemon start
```

With `--output-format json` or `stream-json`, the command also prints a JSON object on standard output. It always has `error` and `request_id`. It has `code` when the daemon gave a code, and `broker_code` when the workspace refused:

```json
{
  "error": "No live shared daemon is available; run biorouter crew daemon start",
  "request_id": "b5e28739-f09f-4398-b6f9-d1f78dd9cbfa"
}
```

Scripts should match on `code` and `broker_code`, not on the wording.

Refusals reach you in two forms:

- A refusal from the workspace is a plain sentence, for example `Only the team's owner or the workspace host can add people to it.` See [How workspace refusals read](#how-workspace-refusals-read).
- A refusal from the daemon on your computer starts with the HTTP status, for example `Error: Daemon returned 409: This session's Crew grant belongs to a different Crew connection.`

### A usage line that lists too much

When a command needs exactly one of two options, a missing option prints a usage line that lists both as required. For example, `biorouter crew invites create @bob` prints:

```text
error: the following required arguments were not provided:
  --team <TEAM>
  --channel <CHANNEL>

Usage: biorouter crew invites create --team <TEAM> --channel <CHANNEL> <PERSON>
```

Give one of them, not both. The same applies to `--text` and `--input` for `tasks start`.

## Naming people, teams, channels and connections

Commands take names. The daemon looks each name up in your own view of the workspace, so it only finds things you can already see. An ID works everywhere a name does.

| You type | It selects | How it matches |
|---|---|---|
| `@bob` | A person | The person's username on the server, exactly. A display name never selects anyone. |
| `analysis-lab` or `"Analysis Lab"` | A team | Teams you belong to, by name or handle. Letter case, spaces, dashes and dots do not matter. |
| `methods` or `'#methods'` | A channel | Your channels in every team. The `#` is optional. |
| `analysis-lab/methods` | A channel in one team | Your channels in that team. |
| `--connection lab` or `--connection bob@hpc.example.edu` | A saved connection | Its name (any letter case) or its SSH target. |
| A UUID, or 64 hexadecimal characters | Anything | Always treated as an ID, never looked up as a name. |

Rules to remember:

- Quote a channel that starts with `#`. In bash, and in zsh with `interactivecomments` set, an unquoted `#methods` starts a comment and the argument disappears. Write `'#methods'` or leave out the `#`.
- There is no current team. Every team has a `general` channel, so `general` alone is ambiguous once you are in two teams. Write `analysis-lab/general`.
- Nothing is guessed. An unknown name prints one line that says so, for example `You're not in a channel called #metods.` An ambiguous name lists the matches you can see and says how to narrow it, for example `Name its team too, like analysis-lab/methods.` The command sends nothing and exits with `1`. If several names are wrong, each gets its own line.
- Usernames must match exactly. `@Bob` for the account `bob` fails with `There's no member @Bob in this workspace. Did you mean @bob? Usernames must match exactly.`
- Some things are never looked up by name. Files, transfers, tasks, chat sessions and remote references always take an ID. Add `--show-ids` to a list command to see them.
- If the daemon is older than the command line, a name fails with `Restart the shared Biorouter daemon to use names; IDs still work.` Stop the daemon and start it again, or pass IDs.

## Start, check and stop the daemon

```bash
biorouter crew daemon status
biorouter crew daemon start
biorouter crew daemon stop
```

- `daemon status` checks the daemon for this profile without asking for the secret. When one is running, it prints `Biorouter daemon running (pid 4242) for this profile.` When none is running, it prints `Error: No such file or directory (os error 2)` and exits with `1`. That message only means no daemon is running.
- `daemon start` asks you to create or type your approval secret, starts the daemon and waits up to 30 seconds for it to be ready. It prints the same line as `daemon status`. You rarely need it, because most other commands start a missing daemon for you.
- `daemon stop` asks for the secret, stops the daemon, and waits up to 30 seconds. It prints `Biorouter daemon stopped.` Stopping it affects every client attached to it, including the desktop app.

Closing the desktop app or ending a command does not stop the daemon. It keeps running in the background until you stop it or restart your computer.

| Message | Meaning and what to do |
|---|---|
| `A shared daemon already owns this profile; connect to it using its existing approval secret` | `daemon start` found one already running. Use it. |
| `biorouterd must be installed alongside biorouter for shared Crew startup` | Install or reinstall Biorouter so both programs sit in the same folder. |
| `Shared daemon exited during startup (...); inspect the profile and installed daemon` | The daemon started and stopped. Check that Biorouter is installed correctly. |
| `The daemon did not establish authenticated shared readiness; only the newly started child was stopped` | The daemon did not become ready within 30 seconds. Try again. |
| `This daemon has no human approval authority; restart it with the trusted Crew terminal launcher` | The running daemon was started without an approval secret, so it refuses every Crew command, `daemon stop` included. Stop that daemon process, then run `daemon start`. `daemon status` shows the same warning. |
| `Stop accepted, but the daemon still owns this profile after 30 seconds; shutdown is unconfirmed. Check daemon status before restarting` | Run `daemon status` and wait until it reports no daemon before you start a new one. |

## Keep device keys in an encrypted vault

Each computer has its own device key for each workspace. By default, Biorouter keeps these keys in your operating system's keyring (the macOS Keychain on a Mac). You can keep them in an encrypted vault protected by a passphrase instead.

```bash
biorouter crew credentials status
biorouter crew credentials init
biorouter crew credentials lock
biorouter crew credentials unlock
```

- These commands need a daemon that is already running. They never start one.
- `credentials status` prints one line. `Credential vault: Not set up · keyring` is the normal state: the vault is not in use and your keys are in the keyring. With a vault, it reads `Credential vault: Locked · encrypted_vault` or `Credential vault: Unlocked · encrypted_vault`.
- `credentials init` and `credentials unlock` ask `Vault passphrase (different from the Crew approval secret):`. The passphrase is 1 to 1024 bytes and must differ from the approval secret.
- Set up the vault before you prepare any identity or save any connection in a new profile. Keys already in the keyring are not moved into the vault.
- A wrong passphrase fails with `Crew vault authentication failed: incorrect passphrase, profile mismatch, or damaged vault`.
- If the vault files are gone, commands fail with `Crew encrypted vault is incomplete or missing; restore its existing files. Keyring fallback and automatic reinitialization are disabled`. Restore the files from your backup. Biorouter does not create a new vault in their place.

## Join a workspace

You need the invitation message your host sent you. It looks like this:

```text
Join lab on Crew.
In Biorouter, open Crew, choose Join a workspace, and paste this whole message.
brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ
```

Save the whole message in a text file, for example `lab-invitation.txt`. The line that starts with `brcrew1:` alone also works. An invitation expires 24 hours after the host invites you.

1. Check what the invitation says. This saves nothing:

   ```bash
   biorouter crew connections join-invitation ./lab-invitation.txt --preview
   ```

   ```text
   Invitation to lab
     Hosted by "Alice Chen" (@alice) on hpc.example.edu
     Workspace privacy: Private · ucsf
     Fingerprint: 6682 327B A040 C709
     Your username on hpc.example.edu: bob
     You'll join as Private · ucsf.
     Connection name on this computer: lab
   ```

   "You'll join as" is how your computer will treat the workspace. The fingerprint identifies the workspace; your host can compare it with the one Crew shows them.

2. Save the connection:

   ```bash
   biorouter crew connections join-invitation ./lab-invitation.txt
   ```

   The command prints the summary again and asks `Save this connection? [y/N]`. Type `y` and press Return. It prints `Saved lab.` and `Next: biorouter crew --connection lab join`.

3. Sign in to the server. Type your SSH password or verification code when the server asks:

   ```bash
   biorouter crew --connection lab auth
   ```

   When it prints `Authenticated. The connection is ready.`, you are connected.

4. Get your code and wait for your host:

   ```bash
   biorouter crew --connection lab join
   ```

   ```text
   "Alice Chen" (@alice) invited you to lab.
   Send Alice this code: 7QK2-M9XA-3JTP-WZ4D
   Waiting for Alice to let you in… Ctrl-C stops waiting; run this command again to continue.
   ```

5. Send the code to your host, for example in Slack or by email. Your computer calculates the code from its own device key and the workspace key in the invitation. Nothing the server sends can change it.

6. When your host enters the code, `join` prints `You're in lab.` and exits with `0`.

If you skip step 3, `join` tries to connect by itself. If it cannot, it fails with a message that starts `Connect to the workspace first: biorouter crew auth`.

### Options for `connections join-invitation`

| Option | What it does |
|---|---|
| `--preview` | Shows the summary and saves nothing. Cannot be combined with `--yes`. |
| `--yes` | Saves without asking. Required when there is no terminal to answer the question, including when the invitation comes from standard input. |
| `--username NAME` | Your username on the server. The default is the username the host invited. |
| `--mode private` or `public` | How this computer treats the workspace. The default is the workspace's own mode. |
| `--institution ID` | Your institution for a Private connection, such as `ucsf`. The default is the workspace's. |
| `--name NAME` | The connection's name on this computer. The default is the workspace's name. |
| `--ssh-target TARGET` | A login from your own SSH settings (an alias, or `user@host`) to use instead of `username@server`. The invitation's port and jump host are then not used. |
| `--port PORT` | The server's SSH port. |
| `--identity-file PATH` | The SSH key file to use. |
| `--proxy-jump ROUTE` | A jump host route. An empty value means no jump host, even when the invitation suggests one. |
| `--remote-root PATH` | A work folder on the server that agents may use for files. Only private models get this access. |
| `--remote-execution` | Also lets agents run commands in that folder. Requires `--remote-root`, and a server that supports the sandbox Crew needs. See [Administration](administration.md). |
| `--preparation-id ID` | For a host saving their own workspace. See [Host a workspace](#host-a-workspace). |

To paste the invitation instead of using a file, pass `-` and end the paste with Ctrl+D. Because there is then no terminal to ask in, the command saves only with `--yes`:

```bash
biorouter crew connections join-invitation - --preview
biorouter crew connections join-invitation - --yes
```

### What `join` can say

`join` checks every 5 seconds and prints a new line only when something changes.

| You see | What it means | What to do |
|---|---|---|
| `Send Alice this code: ...` | You are invited. Your host has not entered your code yet. | Send the code to your host. |
| `The code Alice entered doesn't match this computer. Send it again: ...` | Your host typed a different code. | Send the code again. |
| `You're not in lab yet. Ask "Alice Chen" (@alice) to invite your account on the server.`, then `Waiting for an invitation… Ctrl-C stops waiting.` | The host has not invited your server account. `join` keeps waiting. | Ask the host to run `enroll invite @bob`. |
| `Joining lab…` | Your host entered the right code, and your computer is finishing. | Wait. |
| `You're in lab.` | You joined. For a second computer, the next line is `This computer was added to your account.` | Start using Crew. |
| `This invitation expired. Ask "Alice Chen" (@alice) to invite you again.` | More than 24 hours passed. Exits with `1`. | Ask for a new invitation. |
| `This workspace's server can't let people join with a code yet. Ask the host for an enrollment token instead.` | The server's copy of Crew is too old for codes. Exits with `1`. | See [Enrollment tokens](#enrollment-tokens-for-older-servers). |
| `Restart the shared Biorouter daemon to invite or join with an invitation.` | Your daemon is older than the command line. | Run `daemon stop`, then `daemon start`. |

Press Ctrl+C to stop waiting. The command prints `Stopped waiting. Run biorouter crew join again to continue.` and exits with `0`. Your invitation stays open.

`join --no-wait` prints where joining stands and returns at once, for scripts. It exits with `0` while you are invited, waiting, or not yet invited.

### When saving the invitation fails

| Message | What to do |
|---|---|
| `Saving this invitation needs your username on the server (--username).` | Run the command again with `--username bob`. |
| `Saving this invitation needs the server's address (--ssh-target).` | Add `--ssh-target` with how you reach the server, such as `bob@hpc.example.edu`. |
| `Saving this invitation needs an institution for a Private connection (--institution).` | Add `--institution` with the ID your host gave you. |
| `Add --yes to save this connection: the invitation came from stdin, so there is no terminal to ask in. Run with --preview to check it first.` | You pasted the invitation with `-`. Check with `--preview`, then add `--yes`. |
| `Add --yes to save this connection; there is no terminal to ask in. Run with --preview to check it first.` | You named a file, but the command is not running in a terminal, for example in a script. Check with `--preview`, then add `--yes`. |
| `Nothing was saved.` | You answered something other than `y` or `yes`. Exits with `1`. |
| `Paste the whole invitation your host sent, or the brcrew1: line in it.` | The file or paste was empty. |
| `You already use this server for ... one computer can't mix institutions on the same server.` | Another saved connection reaches the same server under a different institution. Use the same institution, or talk to your host. |

## Sign in and keep the connection up

```bash
biorouter crew status
biorouter crew --connection lab connections show
biorouter crew --connection lab auth
biorouter crew --connection lab connect
biorouter crew --connection lab disconnect
```

- `status` and `connections list` print one line per saved connection: its name, SSH target, status (`Connected` or `Disconnected`) and privacy. A line starting `Last error:` follows when the last attempt failed. With nothing saved, they print `No saved connections.`

  ```text
  lab · bob@hpc.example.edu · Connected · Private (ucsf)
  ```

- `connections show` prints the connection's server, status, privacy, policy epoch, remote folder and SSH key file. `--show-ids` adds its IDs and keys.
- `auth` signs in to the server in your terminal. Type your password and any verification code there. On success it prints `Authenticated. The connection is ready.` and the daemon connects to the workspace. It needs a real terminal: `SSH authentication needs an interactive terminal for native host-key and MFA prompts`.
- `connect` opens the connection without asking for anything. If the server wants a password or a code, it fails with the code `crew_ssh_auth_required`. Run `auth` instead.
- `disconnect` closes the connection. After a `disconnect`, the daemon does not reconnect by itself.

### Reconnecting by itself

The daemon keeps an idle connection alive. When the network drops, it tries again after 20 seconds, 60 seconds and 180 seconds, then every 5 minutes for up to an hour. It does not reconnect by itself after `disconnect`, while the server is waiting for a password or code, after a problem only a person can fix, or after the host removed this computer from the workspace.

If the host removed you with `enroll revoke`, `status` shows the connection disconnected with `Last error: This computer is no longer a member of lab.` JSON output carries `"last_error_code": "crew_membership_ended"`.

### When signing in or connecting fails

`connect` and `auth` report SSH problems with a code. The text reads `Crew SSH failure [CODE; ...]: ...`, and JSON output carries the code in `code`.

| Code | Meaning | What to do |
|---|---|---|
| `crew_ssh_auth_required` | The server wants a password or code. | Run `auth`. |
| `crew_ssh_host_key_unknown` | Your computer has never verified this server. Crew never accepts an unknown server key. | Get the server's key fingerprint from your IT team, add the key to your known hosts file after checking it, then try again. |
| `crew_ssh_host_key_changed` | The server's key differs from the one you verified. | Do not connect. Ask your IT team to confirm the change. |
| `crew_ssh_unreachable` | The server did not answer. | Check your network, or your VPN (the app that connects your computer to your institution's network) if your lab requires one. The daemon keeps trying. |
| `crew_bridge_missing` | `~/.local/bin/biorouter-crew` is missing or cannot run in your server account. | Ask your host or IT team to install it. See [Administration](administration.md). |
| `crew_workspace_identity_mismatch` | The server answered with a different workspace key than your invitation. | Stop. Ask your host what changed. |
| `crew_ssh_failed` | Another SSH failure. | Read the rest of the message, then run `connect` again. |

`auth` itself can end with one of these errors:

| Message | What to do |
|---|---|
| `Signed in, but Crew couldn't start on the server. Crew may not be set up for your account there.` | Signing in worked, but `~/.local/bin/biorouter-crew` is usually missing in your account. Ask your host or IT team to install it. |
| `SSH authentication terminal could not be attached. Close any existing authentication session, verify the daemon's SSH configuration, and try again.` | Close any other `auth` session, or the desktop window where you are signing in to this connection, then try again. |
| `SSH authentication ended before the daemon verified and retained the broker connection` | Signing in ended early, for example because you closed it or the server refused your password. Run `status` to see the last error, then try again. |

For the desktop view of the same problems, see [Connections and troubleshooting](connections-and-troubleshooting.md).

## Change or remove a saved connection

```bash
biorouter crew --connection lab connections update ./lab-connection.json
biorouter crew --connection lab connections remove
biorouter crew connections save ./lab-connection.json
```

- `connections remove` deletes the saved connection from this computer. It does not remove you from the workspace.
- `connections save` saves a new connection from a JSON description, and `connections update` replaces the selected one. Pass `-` to read the JSON from standard input. The input must be UTF-8 and at most 1 MiB.
- Most people never need these. `connections join-invitation` builds the description for you.

The JSON description is one object with these fields. Unknown fields are refused. It never contains a private key or a password.

| Field | Value |
|---|---|
| `name` | The connection's name on this computer. |
| `ssh_target` | Your SSH alias or `user@host`, using your own server account. |
| `port` | Optional SSH port number. |
| `identity_file` | Optional absolute path to your SSH key file. |
| `proxy_jump` | Optional jump host route. |
| `socket_path` | The workspace's absolute socket path on the server. |
| `owner_uid` | The host's numeric user ID on the server. This is not your own ID. |
| `workspace_id` | The workspace's ID. |
| `workspace_public_key` | The workspace's public key, 64 hexadecimal characters. This is not your device key. |
| `mode` | `private` or `public`. The default is `private`. |
| `institution_id` | Required for `private`. Lowercase letters, digits, `_` and `-`, up to 64 characters, such as `ucsf`. |
| `remote_root` | Optional folder on the server for agents. |
| `remote_execution` | Optional, `true` or `false` (default). Lets agents run commands in `remote_root`. |
| `cluster_connection_id` | Optional ID of an existing cluster connection. |
| `preparation_id` | Only when saving a new connection: the ID from `connections prepare`. Leave it out for `update`. |

The output of `connections show` is not a valid description, because it includes state that you cannot set.

## Host a workspace

The host creates the workspace on the server, under their own account. The desktop app can do this for you: in Crew, choose **Host a new workspace**, then **Start it for me**. It runs the same commands over your SSH login. See [Hosting a workspace](hosting-a-workspace.md). The steps below do the same from a terminal.

1. On your computer, prepare this computer's device key and note the key and the preparation ID:

   ```bash
   biorouter crew connections prepare --show-ids
   ```

   ```text
   Device key prepared. Give this public key to the workspace host:
     PUBLIC_KEY_HEX
     Preparation ID: PREPARATION_ID
     Device ID: DEVICE_ID
   ```

   Without `--show-ids`, the preparation ID is hidden.

2. On the server, in your own SSH session, start the workspace. `--name` is the workspace name: 1 to 40 lowercase letters, numbers and dashes, starting and ending with a letter or number.

   ```bash
   "$HOME/.local/bin/biorouter-crew" start --name lab --bootstrap-key 'PUBLIC_KEY_HEX'
   ```

   It prints JSON. Its `invitation` field is a line that starts with `brcrew1:`. Copy the whole output into a file on your computer, for example `lab-start.txt`.

3. On your computer, save your own connection. Give how you reach the server and your institution:

   ```bash
   biorouter crew connections join-invitation ./lab-start.txt \
     --preparation-id 'PREPARATION_ID' --ssh-target alice@hpc.example.edu --institution ucsf
   ```

4. Sign in, then claim the workspace with the key you prepared:

   ```bash
   biorouter crew --connection lab auth
   biorouter crew --connection lab workspace bootstrap
   ```

   `workspace bootstrap` works once, only from your server account and only with the key you gave `start`. It prints `Signed in to lab as "Alice Chen" (@alice).`

5. Confirm the workspace's privacy and institution:

   ```bash
   biorouter crew --connection lab privacy set-workspace private --institution ucsf
   ```

   The institution is permanent. Check it before anyone runs an agent task or grants a chat access. A later change is refused with `Workspace institution cannot be cleared or changed; use a new workspace.`

A new workspace starts as Private. Then invite people, as described in the next section.

### See and rename the workspace

```bash
biorouter crew --connection lab workspace show
biorouter crew --connection lab workspace rename lab-2026
```

`workspace show` prints `Workspace: lab · hosted by "Alice Chen" (@alice)`, the privacy line, `You: ...`, then sections for people, former members, teams, channels, invitations, remote references and agent grants. Empty people, teams and channels sections read `People: none`, `Teams: none` and `Channels: none`.

`workspace rename` is for the host only. It prints `Renamed the workspace to lab-2026.` The workspace keeps its ID, history, invitations and grants.

## Let people into the workspace (host)

```bash
biorouter crew --connection lab enroll invite @bob
biorouter crew --connection lab enroll pending
biorouter crew --connection lab enroll approve @bob 7QK2-M9XA-3JTP-WZ4D
```

### Invite

`enroll invite @bob` checks that `bob` is an account on the server. It looks up that one name and never lists the server's accounts. The output ends with the invitation message to send:

```text
Invited @bob · "Bob Lee" (name on the server account).
Send @bob this invitation:

Join lab on Crew.
In Biorouter, open Crew, choose Join a workspace, and paste this whole message.
brcrew1:...

When @bob sends you a code, let them in with: biorouter crew enroll approve @bob CODE
```

The name in quotes comes from the server account and is only a label. Send the message to Bob in Slack or by email. It contains no secret. Someone you did not invite cannot use it, because joining also needs a server account you invited and your approval of that computer's code.

To print the message again later, run `biorouter crew --connection lab connections invitation --for @bob`. This needs a working connection; otherwise it fails with `Connect to this workspace first, then try again.`

An invitation lasts 24 hours. Inviting the same person again replaces the old invitation. At most 100 people can be waiting at once.

To let an existing member add a second computer, run `enroll invite @bob --add-device`. The new computer joins with its own code.

### Let them in with their code

When Bob sends his code, check who is waiting and enter the code:

```bash
biorouter crew --connection lab enroll pending
biorouter crew --connection lab enroll approve @bob 7QK2-M9XA-3JTP-WZ4D
```

`enroll pending` never shows a code. It lists each person waiting and their state:

```text
Waiting to join (1):
  @bob · "Bob Lee" (name on the server account) · waiting for their code · expires in 23 hours
Let someone in with: biorouter crew enroll approve @USERNAME CODE
```

Type the code as Bob sent it. Letter case, spaces and dashes do not matter. `enroll approve` answers:

```text
Code saved. @bob joins when their computer confirms the same code.
```

Saving a code does not let Bob in yet. The workspace compares the code only when Bob's computer claims it. If you typed it wrong, Bob's `join` says the code does not match, and `enroll pending` shows a warning under his row:

```text
    A computer trying to join as @bob showed a different code. Check the code @bob sent you; if you typed it wrong, run enroll approve again with --replace. Don't approve a code you didn't get from @bob.
```

Only approve a code that the person sent you themselves. To correct a code you typed, add `--replace`:

```bash
biorouter crew --connection lab enroll approve @bob 7QK2-M9XA-3JTP-WZ4D --replace
```

Without `--replace`, a second, different code is refused with `You already entered a code for @bob.` and a line with the command to replace it.

### Withdraw an invitation or remove a member

```bash
biorouter crew --connection lab enroll cancel @bob
biorouter crew --connection lab enroll revoke @bob
```

- `enroll cancel @bob` withdraws the invitation and prints `Cancelled @bob's invitation to join.`
- `enroll revoke @bob` removes Bob from the whole workspace: his membership, all his computers and all agent grants stop working. It asks you to type the username again: `Revoke "Bob Lee" (@bob)? Their membership, devices and agent grants stop working. Type @bob to confirm:`. On success it prints `Revoked "Bob Lee" (@bob). Their membership, devices and agent grants no longer work.`
- In a script, where no terminal can answer, add `--confirm @bob`. `enroll revoke` given a member ID never asks.
- A wrong answer prints `Not revoked: that isn't @bob.` and exits with `1`.
- Removing a member changes the workspace's policy epoch, which ends every agent grant in the workspace. People grant access again afterwards.
- Inviting a removed person again creates a new member with no teams or channels.

### Refusals you may see when inviting

| Message | Meaning |
|---|---|
| `There is no account @zed on this server. Check the spelling.` | No such account. |
| `This server spells the account @alice. Invite @alice.` | The server knows this account by a different spelling. The next line gives the command to run. |
| `@bob is already a member. Choose Add device to add another computer for them.` | Use `enroll invite @bob --add-device` for a second computer. |
| `@root is a system account on this server and can't join a workspace.` | System accounts (root, nobody, accounts below the server's normal user range, accounts that cannot sign in) are refused. |
| `Type the account's name on the server, not its numeric user ID.` | Use the username, not a number. |
| `Another account on this server is already invited as @bob. Cancel that invitation first.` | Run `enroll cancel @bob` first. |
| `100 people are already waiting to join. Cancel an invitation or wait for one to expire.` | Clear some pending invitations. |
| `@eve has no pending invitation. Invite them first.` | You ran `enroll approve` before `enroll invite`. |
| `A code has 16 letters and digits, like 7QK2-M9XA-3JTP-WZ4D. Copy it exactly as they sent it.` | The code is the wrong length. |
| `Workspace host device required.` | Only the host can invite, approve, cancel and revoke. |

### Enrollment tokens for older servers

A server running a copy of `biorouter-crew` built without joining by code cannot use `enroll approve`. On such a server, the older enrollment token path still works, but its commands are hidden from `--help` and print a notice that they are deprecated. The joiner saves a connection and sends the host the device public key, shown on the `Device key:` line of `connections show --show-ids`. The host runs `enroll invite --uid UID --public-key HEX` and sends the printed token privately. The joiner runs `enroll accept` and types the token at the hidden prompt. A token expires after one hour and works once. Update the server's `biorouter-crew` when you can; see [Administration](administration.md).

## Your profile

```bash
biorouter crew --connection lab profile show
biorouter crew --connection lab profile set 'Bob Lee' --avatar 'BL'
```

- `profile show` prints your name and the computers enrolled for you, each with its fingerprint, the date it was added and how it joined.
- `profile set NAME` sets the display name that people see next to your `@username`. `--avatar` sets up to 12 characters of initials or emoji.
- Leaving out `--avatar` removes your initials. To change only the name, pass your current initials with `--avatar` again.
- A display name is a label only. It never gives anyone access. It cannot contain `@` or `#`, and it cannot be another member's username.

## Teams and channels

```bash
biorouter crew --connection lab teams list
biorouter crew --connection lab teams create 'Analysis Lab'
biorouter crew --connection lab teams rename analysis-lab 'Analysis Group'
biorouter crew --connection lab channels list --team analysis-lab
biorouter crew --connection lab channels create methods --team analysis-lab
biorouter crew --connection lab channels rename analysis-lab/methods protocols
biorouter crew --connection lab channels archive analysis-lab/methods
biorouter crew --connection lab channels mark-read methods
```

| Command | What it prints and does |
|---|---|
| `teams create NAME` | `Created team Analysis Lab with #general.` Every new team gets a `general` channel. |
| `teams rename TEAM NAME` | `Renamed Analysis Lab to Analysis Group.` Only the team's creator can rename it. |
| `teams list` | One line per team: its name, its handle when different, the member count and who created it. |
| `channels create NAME --team TEAM` | `Created #methods in Analysis Lab.` If the name was changed to fit the rules, the line adds how, for example that "Data Analysis" was saved as `#data-analysis`. |
| `channels rename CHANNEL NAME` | `Renamed #methods to #protocols.` Only the channel's owner can rename it, and not after it is archived. |
| `channels archive CHANNEL` | `Archived #methods.` The history stays readable, nobody can post, and the name stays taken. Only the owner can archive. |
| `channels mark-read CHANNEL [CURSOR]` | Marks the channel read up to its newest message, or up to a message cursor you give. Prints `Marked #methods as read.` |
| `channels list [--team TEAM]` | One line per channel: name, team, `Restricted` or `Public-safe`, `Archived` when archived, member count, owner and unread count. |

`channels create` has `--classification restricted` (the default) or `--classification public-safe`. Only private AI models can read a Restricted channel. The classification does not limit which people are in the channel. See [Privacy and security](privacy-and-security.md). A team's `general` channel is `Restricted` in a Private workspace and `Public-safe` in a Public one.

Name rules:

- A team name uses letters, numbers, spaces and the characters `- _ . ' & ( ) +`, up to 64 characters. It is unique in the workspace.
- A channel name uses lowercase letters, numbers, hyphens and underscores, up to 80 characters. It is unique in its team, archived channels included. `general` is reserved.
- Two names count as the same when they differ only in letter case, spacing, dashes, dots, invisible characters or letters that look alike. `Analysis Lab` and `analysis-lab` are the same team.
- After ten names that were already taken, within ten minutes, the workspace answers `Too many name attempts. Try again later.` to every create or rename you try. Wait ten minutes, then try again. A name refused for breaking the rules above does not count toward the ten.

## Add people to teams and channels

The host lets people into the workspace. After that, a team's owner or a channel's owner decides who is in it.

### Add someone directly

```bash
biorouter crew --connection lab members
biorouter crew --connection lab members add @bob --team 'Analysis Lab' --channel '#methods'
biorouter crew --connection lab members add @bob --channel analysis-lab/methods
```

- `members` lists everyone, marking `you`, `host`, `former member`, and `account no longer valid` for a server account that was renamed or reused.
- `members add @bob --team TEAM` adds Bob to the team and its `#general`. Each `--channel` also adds him to that channel of the team. Repeat `--channel` for more channels.
- Without `--team`, each `--channel` must be a channel you own in a team Bob already belongs to. The host may name any channel in a team Bob already belongs to.
- Bob does not need to accept. He agreed to take part when he joined the workspace.
- It prints `Added. @bob can now see #general and #methods.`, or `@bob is already in everything you chose.` when nothing changed.
- Without `--team` or `--channel`, it fails with `Choose where to add them: --team for a team you own, or --channel for a channel you own.`
- If one channel of several fails, the error ends with a line telling you to run the same command again with the same `--request-id` to finish.

| Refusal | Meaning |
|---|---|
| `Only the team's owner or the workspace host can add people to it.` | You do not own the team. |
| `You can only add people to channels you own. Uncheck #methods and try again.` | Leave out that channel. |
| `@bob isn't in this channel's team yet. Add them to the team first.` | Add `--team` first. |
| `@bob isn't a member of this workspace any more. Invite them to the workspace first.` | The host must invite Bob again. |
| `This workspace's server can't add people directly yet. Invite them instead: biorouter crew invites create @bob --team <team>` | The server's copy of Crew is older. Use `invites create`. |

### Invite someone to a team or channel

An invitation to a team or channel needs the person to accept it:

```bash
biorouter crew --connection lab invites create @bob --team analysis-lab
biorouter crew --connection lab invites create @bob --channel analysis-lab/methods
```

It prints `Invited "Bob Lee" (@bob) to Analysis Lab. They accept with biorouter crew invites accept.` A channel invitation needs Bob to be in the channel's team already. Give exactly one of `--team` or `--channel`.

Bob accepts:

```bash
biorouter crew --connection lab invites list
biorouter crew --connection lab invites accept
biorouter crew --connection lab invites accept analysis-lab/methods
```

With one pending invitation, `invites accept` needs no argument. With several, name the team or `team/channel`, or the invitation's ID. It prints `Joined Analysis Lab.` or `Joined #methods in Analysis Lab.` With nothing pending, it fails with `You have no pending invitations.` and exits with `1`.

### Remove someone from a channel

```bash
biorouter crew --connection lab remove-member analysis-lab/methods @bob
biorouter crew --connection lab remove-member analysis-lab/methods @carol --former
```

Only the channel's owner can do this, and not to themselves. It prints `Removed "Bob Lee" (@bob) from #methods.` Add `--former` for someone who has already left the workspace. To remove someone from the whole workspace, the host uses `enroll revoke`.

### Hand a channel to someone else

```bash
biorouter crew --connection lab ownership offer methods @carol
```

This prints `Offered #methods to "Carol Nguyen" (@carol). When they accept, they own it and you leave the channel.` Carol then runs:

```bash
biorouter crew --connection lab ownership accept methods
```

It prints `You now own #methods.` The previous owner leaves the channel. A change of owner ends every agent grant in the workspace.

## Messages

```bash
biorouter crew --connection lab send methods --text 'The analysis is ready.'
biorouter crew --connection lab send '#methods' --input ./update.txt
biorouter crew --connection lab history methods --latest --limit 50
biorouter crew --connection lab search analysis-lab/methods 'qPCR' --limit 50
biorouter crew --connection lab watch methods
```

### Post

`send CHANNEL` posts under your own name. Give the text with `--text`, or read it from a file with `--input FILE` (`--input -` reads standard input). It prints `Posted to #methods.`

- `--attachment ID` attaches an uploaded file. `--reference ID` attaches a remote reference. Repeat either one for more. See [Files and remote references](#files-and-remote-references).
- A message can have no text only when it has an attachment or a reference. Otherwise the command fails with `Choose --text, --input, --attachment, or --reference`.
- A message is at most 65,536 bytes.

### Read

`history CHANNEL` prints a page of messages, 100 by default. `--limit` takes 1 to 200.

Without `--latest`, the first page starts from the oldest messages. To see recent messages, add `--latest`.

```text
"Alice Chen" (@alice) · 09:41  The qPCR plates are in the fridge.
"Bob Lee" (@bob) · 09:43  Thanks. Running the analysis now.
    1 attachment
```

Each message starts with the author, `agent` when an agent posted it, and the time in your local time zone. Further lines of a message are indented. An attachment or remote reference appears on its own indented line. With nothing to show, it prints `No messages.`

With `--show-ids`, the page ends with a `Cursor:` line and each message shows its IDs. To page, pass a cursor unchanged to `--before` or `--after`. If the message behind a cursor is no longer visible to you, the command fails with `A message in this view is no longer available to you.`

### Search

`search CHANNEL QUERY` finds messages in one channel that contain the text, ignoring letter case. It takes `--limit` (1 to 200, default 100) and `--after CURSOR`.

### Follow a channel

`watch CHANNEL` prints messages as they arrive, starting from the oldest available message unless you give `--after CURSOR`. Press Ctrl+C to stop. Stopping a watch does not cancel any task.

If the daemon ends the watch, it prints why and exits with `1`, for example:

```text
Error: Stopped watching #methods: You no longer have access to this channel.
```

In JSON modes, it first prints an error object with `"type":"error"` and the reason's code.

Crew sends no system notifications. To follow new messages from a terminal, keep a `watch` running.

## Files and remote references

A file you share is uploaded to the workspace and then attached to a message. Uploading alone does not post anything.

1. Upload the file. Add `--show-ids` to see the transfer ID:

   ```bash
   biorouter crew --connection lab --show-ids files upload methods ./counts.csv
   ```

2. Follow the transfer until it finishes. It prints `Transfer Ready.` for a finished upload:

   ```bash
   biorouter crew --connection lab files watch TRANSFER_ID
   ```

3. Get the attachment ID. With `--show-ids`, the transfer shows a line `  Attachment ID: ...`:

   ```bash
   biorouter crew --connection lab --show-ids files status TRANSFER_ID
   ```

4. Post it:

   ```bash
   biorouter crew --connection lab send methods --text 'Counts attached.' --attachment ATTACHMENT_ID
   ```

To download a file someone posted, find its attachment ID with `history CHANNEL --show-ids`, then:

```bash
biorouter crew --connection lab files download ATTACHMENT_ID --output ./counts.csv
```

### Transfer commands

| Command | What it does |
|---|---|
| `files upload CHANNEL FILE` | Starts an upload to the channel and prints the transfer. |
| `files download ATTACHMENT_ID --output PATH [--overwrite]` | Starts a download to `PATH`. Without `--overwrite`, an existing file is never replaced. |
| `files status TRANSFER_ID` | Shows one transfer. |
| `files watch TRANSFER_ID` | Follows a transfer until it stops. Ctrl+C prints `Stopped watching. The transfer continues.` |
| `files pending` | Lists the transfers for the selected connection. With none, it prints `No file transfers.` |
| `files pause TRANSFER_ID` | Asks the transfer to pause. |
| `files resume TRANSFER_ID FILE [--overwrite]` | Resumes a paused or interrupted transfer. `FILE` is the original upload source or download destination. |
| `files forget TRANSFER_ID [--file PATH]` | Removes the transfer's record. For an unfinished download, give `--file` with its original destination, and the partial file is removed too. |

A transfer line reads, for example, `counts.csv · upload to #methods · Uploading 42% (2.1 MB of 5.0 MB)`. The states are `Starting…`, `Uploading N%`, `Downloading N%`, `Finishing…`, `Pausing…`, `Paused`, `Ready` (upload finished), `Saved` (download finished), `Failed` and `Not confirmed`.

`files watch` exits with `0` when the transfer stops for any reason, including `Transfer Failed.` Read its last line.

Limits and safety rules:

- A file is at most 1 GiB. A workspace holds at most 10 GiB of files and 10,000 files.
- The download folder must belong to you and must not be writable by other accounts: `Choose an output directory owned by your account that other accounts cannot modify`.
- Crew refuses to save into credential or settings folders: `Crew won't save into a credential or settings location. Choose another folder.`
- Crew refuses to upload a file that looks like a password, key or token store: `“secrets.yaml” looks like a credential file (a password, key or token store), so Crew won't share it.`
- An existing destination without `--overwrite` fails with `Destination exists; explicitly approve replacement or select another filename`.

### Remote references

A remote reference points to a path on the server without uploading anything:

```bash
biorouter crew --connection lab --show-ids files reference methods '/project/analysis/results' --label 'Remote results'
biorouter crew --connection lab files show-reference REFERENCE_ID
biorouter crew --connection lab send methods --reference REFERENCE_ID
```

Crew does not check or read the path. The label is at most 255 bytes. A reference line ends with `Not uploaded`.

## Agent tasks

A task runs your own AI agent on a prompt, lets it read the channel, and posts its result in the channel.

```bash
biorouter crew --connection lab tasks start methods \
  --input ./task-prompt.txt --provider PROVIDER --model MODEL --allow-posting
biorouter crew --connection lab tasks list --show-ids
biorouter crew --connection lab tasks show TASK_ID
biorouter crew --connection lab tasks watch TASK_ID
biorouter crew --connection lab tasks cancel TASK_ID
```

### Start a task

| Option | What it does |
|---|---|
| `CHANNEL` | The channel the task reads and posts its result to. |
| `--text TEXT` or `--input FILE` | The prompt. Give exactly one. `--input -` reads standard input. |
| `--provider NAME` | The AI provider, as Biorouter names it in your configuration (`BIOROUTER_PROVIDER` in `~/.config/biorouter/config.yaml`). |
| `--model NAME` | The model name. |
| `--context-channel CHANNEL` | Another channel the task may read. Repeat for more, up to 16. More than 16 fails with `Daemon returned 400: Select at most 16 additional channels.` |
| `--allow-posting` | Required. Without it: `Starting a Crew task requires --allow-posting for its destination channel`. |

A task's access to the workspace lasts one hour.

In a Private workspace, the model must be approved for the workspace's institution, or run locally. Otherwise the task is refused before it starts, for example `gpt-5.5 is approved for ucsf. partner-lab uses stanford. Choose a model approved for it, or a local model.` JSON output carries the code `crew_request_refused`.

The result the task posts ends with a line Biorouter adds about the files the task read, for example ``Source: `counts.csv`, shared by Bob Lee (@bob).``, or `No shared file was read for this result.`

### Follow and cancel

- `tasks list` prints one line per task, such as `Task "Summarize the qPCR results" posting to #methods · Working…`, then `  Chat: SESSION_ID`. With no tasks, it prints `No tasks.`
- The status words are `Starting…`, `Working…`, `Waiting for your approval`, `Stopping…`, `Stop not confirmed`, `Interrupted`, `Outcome unknown`, `Done`, `Couldn't finish` and `Stopped`.
- `tasks watch` checks every 2 seconds and stops when the task reaches `Done`, `Couldn't finish`, `Stopped`, `Interrupted`, `Outcome unknown` or `Stop not confirmed`. It exits with `0` in every one of those cases, so read the last line. Ctrl+C stops watching; the task keeps running.
- `tasks cancel` normally prints `Local cancellation requested and remote grant revoked. Remote process termination was not confirmed.` If the task had already ended, it prints `The task had already finished: ...`.
- If the workspace cannot be reached, `tasks cancel` exits with `1`, says the remote grant revocation is unconfirmed, and ends with `Retry safely with --request-id ...`. Run it again with that ID.
- `Task was not found on this device and connection` means the ID is wrong, or the task belongs to another connection.

## Give a chat access to Crew

You can let an ordinary Biorouter chat read channels and post in one channel. The desktop app does this with `/crew`; see [Agents and chat access](agents-and-chat-access.md). From the terminal, you need the chat's session ID, such as `20260924_2`. `biorouter session list` prints each chat's ID, name and last update.

The chat must be saved on this computer, open in the shared daemon, and idle. Open it in the desktop app and wait until its reply finishes, then run:

```bash
biorouter crew --connection lab grants grant SESSION_ID methods
biorouter crew --connection lab grants grant SESSION_ID methods --context-channel analysis-lab/raw-data
```

It prints `Crew access granted to chat SESSION_ID.` The first channel is where the chat may post; each `--context-channel` is another channel it may read. A chat reads at most 20 channels, including the one it posts to. This limit differs from the 16 extra channels a task may read. If the chat is busy or not open, the command fails with `Wait for the conversation's current turn to finish before granting Crew access.` or `Open the conversation before granting Crew access.`

### Check access

```bash
biorouter crew --connection lab grants list
biorouter crew --connection lab context SESSION_ID
```

`grants list` prints one line per chat or task with access:

```text
Chat "qPCR notes" (chat 20260924_2) → #methods in Analysis Lab · Active · ends in 42 minutes · policy epoch 7
```

| State | Meaning |
|---|---|
| `Active` | The chat or task may use Crew. |
| `Expired` | A chat's access ran out. |
| `Ended` | A task's access ended with the task. |
| `Revoked` | You revoked it, and the workspace confirmed. |
| `Stopped on this device; the workspace hasn't confirmed yet` | You revoked it, and the workspace has not confirmed yet. Biorouter keeps asking by itself. |
| `Ended: Crew settings changed` | The workspace ended it, because its settings or membership changed. See [Why access ends by itself](#why-access-ends-by-itself). |

With nothing granted, it prints `No chats have Crew access.` A second heading, `Earlier access no chat holds any more:`, lists older grants whose revocation Biorouter is still confirming.

`context SESSION_ID` shows which channels a grant covers.

### Why access ends by itself

Some changes raise the workspace's policy epoch, and a higher epoch ends every grant and task in the workspace at once.

These changes raise the policy epoch:

- adding someone to a team or channel
- accepting a team or channel invitation
- removing someone from a channel
- archiving a channel
- offering or accepting a channel's ownership
- removing a member from the workspace
- changing the workspace's privacy

The next time an affected chat uses Crew, it is refused with `Crew settings changed since access was granted. Grant access again from Crew.`, and `grants list` shows `Ended: Crew settings changed`. Grant access again when you need it.

### End access

```bash
biorouter crew --connection lab grants revoke SESSION_ID
```

| Result | What it prints | Exit status |
|---|---|---|
| The workspace confirmed | `Access revoked. Chat SESSION_ID can't use Crew until you grant access again, or start a new chat.` For a task, also `Its task was stopped.` or its status. | `0` |
| Stopped here, not yet confirmed | `Stopped on this device. The workspace hasn't confirmed the revocation yet; Biorouter confirms it with the workspace by itself when the connection is back. biorouter crew grants list shows when it has.` | `1` |
| Not revoked | `Not revoked. This chat can still read and post. Check biorouter crew grants list, then try again.` | `1` |
| No grant | `Daemon returned 404: No Crew grant for this session.` | `1` |
| Wrong connection | `Daemon returned 409: This session's Crew grant belongs to a different Crew connection.` Pass the right `--connection`. | `1` |
| Granted again meanwhile | `Daemon returned 409: This session was granted Crew access again while its previous grant was being revoked. The new grant is active; revoke again to stop it.` | `1` |

When the result is "stopped here, not yet confirmed", this computer's chat already cannot use Crew. You do not need to run anything else. Biorouter asks the workspace again each time it connects, and at growing intervals while connected. Running `grants revoke` again asks at once.

After a revocation, the chat's next Crew request is refused with `This chat's Crew access was removed. Start a new chat, or grant access again from Crew.` After the workspace ends a grant, the message is `Crew settings changed since access was granted. Grant access again from Crew.`

A revoked grant stops the chat from using Crew. It does not prove that a command already running on the server has stopped.

### Use a terminal chat with Crew

You can create a chat in the shared daemon from the terminal, grant it access, then talk to it:

```bash
biorouter session --shared-daemon --no-start --create-only --provider PROVIDER --model MODEL
biorouter crew --connection lab grants grant SESSION_ID methods
biorouter session --shared-daemon --no-start --resume --session-id SESSION_ID
```

The first command prints the new chat's session ID. Use it in the next two. For a single prompt instead of an interactive chat, use `biorouter run --shared-daemon --no-start --resume --session-id SESSION_ID --text 'Summarize this week in #methods.'`. Without `--shared-daemon`, `biorouter session` and `biorouter run` start a separate local chat that cannot use a Crew grant.

## Privacy

```bash
biorouter crew --connection lab privacy show
biorouter crew --connection lab privacy set-personal private --institution ucsf
biorouter crew --connection lab privacy set-workspace private --institution ucsf
```

`privacy show` prints your connection's privacy, the workspace's privacy, each channel's classification and the two policy epochs:

```text
Your connection: Private · institution ucsf · policy epoch 3
Workspace: Private for everyone · institution ucsf · policy epoch 7
Channels (2):
  #general · Analysis Lab · Restricted
  #methods · Analysis Lab · Restricted
Pass the policy epochs to --expected-policy-epoch and --expected-workspace-policy-epoch to refuse a changed policy.
```

| Command | Who | What it does |
|---|---|---|
| `privacy set-personal private` or `public` | Anyone | Changes how this computer treats the workspace. `--institution ID` is required when you set a new Private label. It prints the updated connection. |
| `privacy set-workspace private` or `public` | Host | Changes the workspace's privacy. `--institution ID` confirms the workspace's institution, which can never change once set. It prints `Workspace privacy: ...` with the new policy epoch. |

An institution ID is 1 to 64 lowercase letters, digits, `_` or `-`, starting with a letter or digit.

Changing the workspace's privacy raises its policy epoch and ends every agent grant. Switching to Public never makes earlier Private content public. What Private, Public, `Restricted` and `Public-safe` mean is explained in [Privacy and security](privacy-and-security.md).

### Guard a script against a policy change

The policy epoch is a number that goes up whenever the privacy settings or the membership change. A script can refuse to act if the policy changed after it checked:

```bash
biorouter crew --connection lab --expected-policy-epoch 3 \
  --expected-workspace-policy-epoch 7 tasks start methods \
  --input ./task-prompt.txt --provider PROVIDER --model MODEL --allow-posting
```

Take the numbers from `privacy show`. These two options apply only to `tasks start` and `grants grant`. For posts and file transfers, use `--expected-mode private` instead.

## Retry after an uncertain result

Sometimes a change reaches the workspace but the answer does not reach you, for example when the network drops. Then the error ends with a line such as:

```text
Retry safely with --request-id 3f0c9a0e-5a1b-4c8e-9d2f-6b7a8c9d0e1f
```

Run the same command again with that option. The workspace recognizes the ID and does not make the change twice. This line appears only when the result is uncertain. A clear refusal never shows it.

- Every command gets a new request ID unless you pass one. JSON output always includes `request_id`.
- An ID is 1 to 128 letters, digits, underscores or hyphens. The command refuses any other ID before it asks for anything.
- Use a new ID for a different change. Reusing an ID for a different upload or download fails with `Request ID belongs to a different transfer; inspect its receipt before choosing a new request ID`.
- To resume an interrupted transfer, use `files resume`. Retrying the upload command does not resume it.

## Recover after a restart

After your computer or the daemon restarts, do not assume that tasks or transfers carried on. Check each thing in order:

1. Run `biorouter crew daemon status`. If no daemon is running, any other command starts one.
2. If you use the encrypted vault, run `biorouter crew credentials unlock`.
3. Run `biorouter crew status`. If a connection is `Disconnected`, run `auth` or `connect`.
4. Run `files pending`, `tasks list` and `grants list`.
5. For a task that shows `Interrupted`, `Outcome unknown` or `Stop not confirmed`, read its channel before you start it again. Run `tasks cancel` again, or `grants revoke`, as the message says.

If a command reports a changed server key, a changed workspace key, or that it cannot verify the daemon, find out why before you connect again. Do not replace keys or saved files to make the message go away.

## How workspace refusals read

When the workspace refuses a request, the terminal shows a sentence instead of the server's internal code. JSON output keeps the code as `broker_code`. Common ones:

| You see | Meaning |
|---|---|
| `This computer isn't a member of this workspace.` | The workspace does not know this computer. The host may have removed it. |
| `This computer isn't signed in to this workspace.` | Sign in again with `auth`. |
| `You're no longer a member of this workspace.` | The host removed you. |
| `That channel isn't available to you. It may be archived, or you may not be in it.` | Check the channel name and your membership. |
| `That person isn't a member of this workspace.` | Check the username. |
| `This task's access to the workspace has ended.` | The task's grant ended. |
| `The workspace didn't allow this.` | You lack the role for this action, such as host or owner. |
| `That name is already taken.` | Choose another name. |
| `Too many attempts at once. Wait a minute, then try again.` | The workspace is limiting requests. |
| `This workspace has grown past the size Crew supports and cannot take more changes. Ask the host about starting a new workspace.` | The workspace is full. Reading still works. |
| `Account enrollment changed.` | Your server account was renamed or replaced since you joined. Ask the host to remove and invite you again. |
| `The workspace refused this request.` | Any other refusal. Run the command with `--output-format json` and give the `broker_code` to your host. |

## Server commands

The server program `biorouter-crew` runs in the host's account on the Linux server. It is not part of `biorouter crew`. Its help reads:

```text
Usage: biorouter-crew start --name NAME --bootstrap-key HEX [--state-dir PATH]
       biorouter-crew serve|start|status|stop --state-dir PATH [--name NAME] [--bootstrap-key HEX]
       biorouter-crew bridge --stdio --socket PATH --owner-uid UID --workspace-id UUID
       biorouter-crew --version
```

| Command | Use |
|---|---|
| `start --name NAME --bootstrap-key HEX` | Start a new workspace in the background and print its invitation. |
| `start --state-dir PATH` | Start an existing workspace again, for example after the server restarts. |
| `status --name NAME` | Check that the workspace is running. NAME is the folder name the workspace was created with. |
| `stop --name NAME` | Stop the workspace. Only the host's account can stop it. NAME is the folder name the workspace was created with. |
| `--version` | Print the installed version, such as `biorouter-crew 1.91.2`. |

Each workspace keeps its state in a folder under `~/.local/share/biorouter-crew`. The folder has the name the workspace was created with, and keeps that name after `workspace rename`. Run `ls "$HOME/.local/share/biorouter-crew"` to list the folders.

Crew installs no service that starts the workspace when the server boots. After a reboot, the host starts it again from its folder, without `--name`:

```bash
"$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/<original name>"
```

Do not add `--name` to this command. After a rename, the old name is refused with `name_mismatch: this workspace already has another name; start it without --name, or rename it in Crew`. The new name points at a folder that does not hold the workspace, so `start` fails with a `bootstrap_key` error. The steps are in [After the server restarts](hosting-a-workspace.md#after-the-server-restarts) in Hosting a workspace.

Members never run `bridge`; the daemon on their computer runs it over their SSH login. Server requirements, installation, upgrades and backups are in [Administration](administration.md).

## Common mistakes

| Mistake | What happens and what to do |
|---|---|
| Typing `#methods` without quotes | The shell treats it as a comment and drops it. Write `'#methods'` or `methods`. |
| Reading `history` without `--latest` | You get the oldest page. Add `--latest` for the newest. |
| Expecting `files upload` to post | It only uploads. Post the attachment ID with `send --attachment`. |
| Looking for the preparation ID | `connections prepare` hides it. Add `--show-ids`. |
| Treating `enroll approve` as letting someone in | It saves the code. The person is in only when their `join` prints `You're in lab.` |
| Reading `Credential vault: Not set up · keyring` as a problem | Your keys are in the keyring, which is the default. |
| Reading `No such file or directory (os error 2)` from `daemon status` as damage | No daemon is running. Any other command starts one. |
| Installing `biorouter-crew` only in `/usr/bin` | Each account needs its own copy at `~/.local/bin/biorouter-crew`. |
| Writing `--classification public_safe` | The option value is `public-safe`. Only JSON output spells it `public_safe`. |
| Expecting `watch`, `tasks watch` or `files watch` to exit with `1` on a failed result | They exit with `0` when they stop. Read the last line. |

## Related documentation

- [Crew user manual](README.md): what Crew is and where each topic lives.
- [Getting started](getting-started.md): what you need before using Crew, and the words this manual uses.
- [Hosting a workspace](hosting-a-workspace.md): the desktop steps for creating a workspace and letting people in.
- [Joining a workspace](joining-a-workspace.md): the desktop steps for joining with an invitation.
- [Teams, channels and people](teams-channels-and-people.md): teams, channels, owners and display names in the desktop app.
- [Messages and files](messages-and-files.md): reading, posting and sharing files in the desktop app.
- [Agents and chat access](agents-and-chat-access.md): agent tasks and chat access in the desktop app.
- [Privacy and security](privacy-and-security.md): Private and Public, institutions, channel classifications and device keys.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connection states, server identity checks and error messages.
- [Administration](administration.md): server requirements, installing `biorouter-crew`, data locations, limits, upgrades and backups.
- [Crew CLI guide for developers](../research/biorouter-crew/cli-guide.md): the design notes behind these commands, written for developers and testers.
