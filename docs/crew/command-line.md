# Crew command line

> **What this is.** A task reference for the `biorouter crew` commands: their syntax, what they print and their exit status.
> **Status:** Current. Checked against the Biorouter 1.93.0 Crew code on 2026-10-16.
> **Audience:** IT staff, lab managers, and lab members who prefer a terminal or want to script Crew. You should know how to open a terminal and run a command.

These commands and the desktop app share one background service on your computer, the Biorouter daemon (`biorouterd`), and its saved connections, so both show the same workspace.

Every command here starts with `biorouter crew`, which short examples leave out. Add `--connection lab` when several connections are saved; without it, such a command exits with `2` and names the option. The examples use workspace `lab`, host `@alice`, new member `@bob`, team `analysis-lab` and channel `#methods`. Capitals mark placeholders. `--help` lists every option.

## Before you start

You need:

- A Mac or Linux computer. On Windows, every command stops before it asks for anything, with `Shared Crew daemon IPC is unavailable on this platform.`
- `biorouter` and `biorouterd` in the same folder, and the same Biorouter profile as your desktop app.
- An account on the lab's Linux server, with an SSH key, password or code from your IT team.
- Nothing installed on the server: `setup`, `host` and `join` install `~/.local/bin/biorouter-crew` in your own account there ([Set up Crew on a server](#set-up-crew-on-a-server)).
- The server's SSH host key in your known hosts file. See [Connections and troubleshooting](connections-and-troubleshooting.md).
- On a Linux computer with no keyring service, such as a login node you reach only over SSH, an encrypted vault set up first. See [Keep device keys in an encrypted vault](#keep-device-keys-in-an-encrypted-vault).

If this computer is the server that runs the workspace, also read [Join from the workspace's own server](#join-from-the-workspaces-own-server).

## The daemon starts by itself

The commands connect to the daemon without asking you for anything. When no daemon is running, the first command that needs one starts it. The desktop app then connects to the same daemon.

A daemon started by an older version of Biorouter may not accept these commands. A command then stops it and starts a new one. With `--no-start`, the command refuses instead, with code `crew_daemon_needs_restart` and `The running Biorouter daemon was started by an older version of Biorouter, so this command cannot use it. Run biorouter crew daemon stop, then run this command again.` Your connections, keys and workspaces stay. A running task or transfer may stop, so follow [Recover after a restart](#recover-after-a-restart).

### Run commands from a script

To send the vault passphrase for `credentials init` or `credentials unlock`, add `--passphrase-stdin` and send the passphrase as the first line of standard input, for example `print-passphrase | biorouter crew --passphrase-stdin credentials unlock`. A script sends a new passphrase once; only a terminal asks for it twice, so the script is responsible for sending the one it means. Never put the passphrase in an argument, shell history, environment variable or file.

`--approval-key-stdin` is still accepted so older scripts keep working. The command reads the first line of standard input and ignores it, and any further input, such as a passphrase or `send --input -` text, follows on the next lines.

`auth` always needs a real terminal. Without one it exits with `2` (code `crew_needs_terminal`), and says that `connect` signs in without one when the server takes this computer's SSH key alone.

## Global options

These work before or after the command name.

| Option | Effect |
|---|---|
| `--connection NAME` | Chooses a saved connection by name, SSH target or ID. |
| `--show-ids` | Adds IDs to text output. JSON always has them. |
| `--output-format json` or `stream-json` | Prints JSON, indented or one value per line. |
| `--no-start` | Fails instead of starting a daemon. |
| `--passphrase-stdin` | Reads the vault passphrase from the first line of standard input. See [Run commands from a script](#run-commands-from-a-script). |
| `--approval-key-stdin` | Accepted for older scripts. Reads the first line of standard input and ignores it. |
| `--request-id ID` | Reuses the ID of a change whose result was uncertain. See [Retry after an uncertain result](#retry-after-an-uncertain-result). |
| `--expected-mode private` or `public` | Refuses `send`, `tasks start`, `grants grant` and file transfers unless the mode you name is the privacy in force, the one `status` shows, or your connection's own setting. |
| `--expected-policy-epoch N`, `--expected-workspace-policy-epoch N` | Refuse `tasks start` and `grants grant` when the policy changed. See [Privacy settings](#privacy-settings). |

## Output and exit status

Results go to standard output, and prompts and error sentences to standard error. With `--output-format json` or `stream-json`, a failed command also prints its error as a JSON object on standard output, so a script reads one place. `watch`, `join`, `tasks watch` and `files watch` print one JSON value per line in both JSON formats, their last value and their error included.

| Exit | Meaning |
|---|---|
| `0` | Success, or you stopped one of those four with Ctrl+C. |
| `1` | Failed, refused, or not finished, such as `grants revoke` before the workspace confirms. |
| `2` | Wrong command line, a command that needed a terminal to ask you something ran without one, several saved connections and no `--connection`, or a message too long to send. Nothing was sent. |

In JSON, an error has `error` and `request_id`, and usually a `code`. The code is your daemon's, such as `crew_outcome_unknown` (see [Workspace refusals](#workspace-refusals)), or one the command line gives its own errors, in the table below. A refusal from the workspace adds `broker_code`, and a refused `connect` adds `detail`. A daemon refusal also keeps its other fields, such as `reason`, `actual_mode`, `expected_mode` or `institution_refusal`. Scripts should match those codes and fields, not the wording. A daemon refusal prints its own sentence; only one that has no sentence of its own starts with its HTTP status, such as `Daemon returned 409:`. When a command needs one of two options, the usage line lists both as required. Give exactly one.

| Code | What it means |
|---|---|
| `unknown_name`, `ambiguous_name` | A name the command could not look up. See [Name people, teams and channels](#name-people-teams-and-channels). |
| `crew_lookup_failed` | The daemon's answer to a name lookup was not one the command could use. Run it again, or use the ID. |
| `crew_needs_terminal` | The command needed a terminal to ask you something. Exit `2`. |
| `crew_connection_required` | Several connections are saved and none was chosen: ``Several Crew connections are saved; choose one with --connection NAME. Run biorouter crew connections list to see them.`` Exit `2`. |
| `crew_message_too_long` | The message is over 64 KB. See [Post and read messages](#post-and-read-messages). Exit `2`. |
| `crew_nothing_to_replace` | `connections join-invitation --replace` found no saved connection it may replace. Exit `2`. |
| `crew_daemon_not_running` | No daemon runs for this profile. See [Start, check and stop the daemon](#start-check-and-stop-the-daemon). |
| `crew_daemon_needs_restart` | With `--no-start`, the running daemon was started by an older version of Biorouter. Run `daemon stop`, then the command again. |
| `crew_platform_unsupported` | This computer runs Windows, where the commands do not work. |
| `crew_no_vault` | `credentials unlock` found no vault to unlock, because this computer keeps Crew keys in its keyring. |
| `crew_not_sent` | The request never left this computer. Run it again as it was. |
| `crew_host_required` | Only the workspace host can run this command, such as `enroll pending` in a managed workspace, `workspace delete` or `workspace governance`. Nothing was sent. |
| `crew_institution_required` | `host --mode private` needs `--institution ID`. Exit `2`. |
| `crew_folder_required` | `connections set-access folder` or `full` needs a work folder, and the connection has none: add `--folder PATH`. Exit `2`. |
| `crew_task_model_required` | `tasks start` or `ask` named no model, and none is set for Crew tasks or new chats. Add `--provider` and `--model`, or run `defaults set`. Exit `2`. |

Text output prints a display name in double quotes, such as `"Alice Chen" (@alice)`. A name with letters outside ASCII also has two invisible direction marks (U+2068 and U+2069) inside its quotes, so right-to-left text cannot reorder the rest of the line. Scripts should read names from JSON, which has neither the quotes nor the marks.

## Name people, teams and channels

The daemon looks names up in your own view of the workspace. An ID works anywhere a name does.

- `@bob` is the person whose server username is exactly `bob`. A display name never selects anyone.
- A team is its name or handle, such as `analysis-lab` or `"Analysis Lab"`. Case, spaces and dashes do not matter.
- A channel is `methods`, `'#methods'` or `analysis-lab/methods`. Quote a leading `#`, or the shell drops the word. Once you are in two teams, write `analysis-lab/general`.
- Files, transfers, tasks, chat sessions and remote references take only IDs. `--show-ids` shows them.

Nothing is guessed. An unknown name says so, such as `No channel you're in is called #methods.`, whether you mistyped it, it was renamed or you left it. An ambiguous one lists the matches. A channel ID from another workspace, or of a channel you are not in, is refused the same way, with `No channel you're in has the ID …`, by the commands only a member runs. Nothing is sent, the command exits with `1`, and JSON carries `unknown_name` or `ambiguous_name`.

A rename changes the name you type. After a team is renamed, its old name and handle no longer find it, so use the new name, which `teams list` shows. The same holds for a channel.

In a very large workspace, the daemon may list fewer of your teams and channels than you are in. List commands then end with a line such as `Showing 100 of your 140 channels.`, and a name that an unlisted team or channel might also have is ambiguous: its matches end with `Possibly others: you're in more teams and channels than Biorouter can list at once`. Write the team too, such as `analysis-lab/methods`, or use the ID.

## Start, check and stop the daemon

- `daemon status` prints the pid, or exits with `1` and `No Biorouter daemon is running for this profile.` (code `crew_daemon_not_running`), also when a daemon that crashed left its files behind.
- `daemon start` starts the daemon. While one is running it refuses with `A Biorouter daemon is already running for this profile. Stop it first with biorouter crew daemon stop.` Most commands start the daemon for you.
- `daemon stop` stops it for the desktop app too. Closing the app does not stop it. An open desktop app starts a new one right away and reconnects by itself.

A message that starts `Restart the shared Biorouter daemon` means the daemon is older than the command, so run `daemon stop`, then the command again. For the same problem in the desktop app, see [Replace an old background service](connections-and-troubleshooting.md#replace-an-old-background-service). After a message that starts `Stop accepted, but`, wait until `daemon status` shows no daemon.

## Keep device keys in an encrypted vault

Each computer has a device key for each workspace, kept in your system keyring by default. On a Mac, or a Linux desktop where you are signed in, `credentials status` then prints `Credential vault: Not set up · keyring`, which is normal. To use a passphrase vault instead, run `credentials init` before you prepare a key or save a connection. Keys already in the keyring do not move.

A Linux computer with no desktop session, such as a login node you reach only over SSH, often has no keyring service (Secret Service). `credentials status` still prints `Not set up · keyring` there, but Crew cannot save a key, and Crew never keeps a device key in a plain file. There, run `credentials init` first, before `connections join-invitation`, `connections prepare` or `connections save`. Without a vault, the first of those fails with ``This computer has no keyring service Biorouter can use. Run `biorouter crew credentials init` to keep Crew keys in an encrypted vault, then try again.`` (code `crew_credential_store_unavailable`). After `credentials init`, `credentials status` prints `Credential vault: Unlocked · encrypted_vault`.

`credentials init` works only in a Crew profile that holds no keys yet. A keyring that stops answering after you have joined gives a sentence asking you to start it again, for example by signing in to the computer's desktop. A keyring that is locked or that refused access (code `crew_credential_store_refused`) asks you to unlock it or allow access.

- The passphrase is 1 to 1024 bytes. `credentials init` asks for it twice and sets up nothing if the two differ, because a vault nobody can unlock loses the keys in it.
- `credentials lock` and `credentials unlock` close and open the vault. Like other Crew commands, they start the daemon if needed; `--no-start` refuses instead.
- If the vault files go missing, restore them from backup. Biorouter never replaces them with a new vault.

## Join a workspace

Save the invitation your host sent, or only its `brcrew1:` line, in a file such as `lab.txt`. It expires 24 hours after the host invites you.

1. Run `biorouter crew connections join-invitation ./lab.txt --preview`. It saves nothing. `You'll join as` shows how your computer will treat the workspace. Your host can compare the fingerprint.
2. Run it again without `--preview`. Type `y` and press Enter. It prints `Saved lab.` and the next commands to run.
3. Run `biorouter crew setup`. It signs in, asking for your password or code in the terminal if the server wants one, installs Crew in your server account if it is missing, gives agents full access with a work folder as the desktop's Join does ([Set up Crew on a server](#set-up-crew-on-a-server)), and connects. It ends with `Connected.`
4. Run `biorouter crew join`. It prints `Send Alice this code: 7QK2-M9XA-3JTP-WZ4D` and waits. When Crew is missing from your server account, or the server wants you to sign in at the terminal, `join` runs the same setup first. If this computer has never connected to the workspace, that setup also gives agents full access, as step 3 does, and warns you once. If it has connected before, `join` keeps what agents may do there.
5. Send the code to your host, for example by Slack or email.
6. When the host enters it, `join` prints `You're in lab.` If agents still have no access on the server, it adds ``Your agent can't work on hpc yet. To let it, run biorouter crew --connection lab connections set-access full.`` ([Agent access](#agent-access)).

To paste the invitation, pass `-`, end with Ctrl+D, and add `--yes`. If saving fails, the message names the option to add. One computer cannot use two institutions on one server.

An invitation names one server account. If this computer signs in to the server as another account, the summary says so, such as ``This invitation is for @bob, but this computer signs in to lab-ubuntu as carol. Ask your host for your own invitation.`` Saving it anyway works, but that connection can never join. Your own invitation for that workspace then gets, in the preview, ``This computer already has lab for this workspace, signing in as another account, and it has never connected. Add --replace to save this invitation in its place.``, and without `--replace` the save is refused with ``This computer already has lab for this workspace, signing in as another account, and it has never connected. Run it again with --replace, or remove it with biorouter crew --connection lab connections remove.`` Run `join-invitation` again with `--replace` to save your invitation in place of that connection.

For a workspace this computer already has, the preview says ``This computer already has this workspace as lab; saving again keeps that connection if its settings match.`` A save with other settings, such as another server login, key file or privacy, is refused with ``This workspace is already saved as lab with a different server login, port, key file, jump host or privacy, so nothing was saved.`` The lines after it name each way on: `connections show` to compare the two, `privacy set-personal` to give the saved connection the invitation's privacy, `connections update FILE` to change its server details, `connections remove` and a new save if it never joined, or `join` to use it as it is. See [Change or remove a saved connection](#change-or-remove-a-saved-connection).

If the server takes this computer's SSH key alone, step 3 can also be `auth` (which signs in and connects) or `connect`, followed by `setup` when Crew is not there yet.

`--mode`, `--institution`, `--username` and `--name` override the invitation's values. `--ssh-target` uses a login from your SSH settings and ignores the invitation's port and jump host. Without `--ssh-target`, `--port` or `--proxy-jump`, the login saved is your username at the invitation's server, such as `bob@hpc.example.edu`. When a `Host` entry in your `~/.ssh/config` signs in as that same account to the same server on the same port, Crew saves the entry's name instead, so the key and jump host you set there apply. An entry Crew could not connect through, such as one with a `ProxyCommand` or a `RemoteCommand`, is passed over. `--port`, `--identity-file` and `--proxy-jump` set SSH details. A relative `--identity-file`, or one starting with `~/`, is made absolute, and an empty `--proxy-jump` means none. `--remote-root PATH` gives agents a server work folder, for private models only, and `--remote-execution` lets them run commands there. To give them full access instead, run `setup` or `connections set-access full` after saving ([Agent access](#agent-access)). See [Administration](administration.md).

While `join` waits:

| You see | What to do |
|---|---|
| `The code Alice entered doesn't match this computer. ...` | Send the code again. |
| `You're not in lab yet. Ask "Alice Chen" (@alice) to invite your account on the server.` | Ask the host to run `enroll invite @bob`. |
| `This invitation expired. ...` | Ask for a new invitation. |
| `This workspace's server can't let people join with a code yet. ...` | See [Tokens for older servers](#tokens-for-older-servers). |

Ctrl+C stops waiting and keeps the invitation open. `join --no-wait` prints the state and returns, for scripts.

The first time you pass through a jump host, verify it on its own first, as step 3 of [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer) shows. IT's settings make SSH check jump hosts strictly, so SSH never asks about an unknown one.

### Join from the workspace's own server

If you run `biorouter crew` on the server that runs the workspace, such as a lab login node, Crew still reaches the workspace over SSH, from your account to your own account on the same machine. It needs no jump host, and two things SSH does not set up by itself.

1. If this computer has no keyring service, run `credentials init` first ([Keep device keys in an encrypted vault](#keep-device-keys-in-an-encrypted-vault)).
2. Let your own key sign you in. If `~/.ssh/id_ed25519.pub` does not exist, run `ssh-keygen -t ed25519` and press Enter at each question. Then run `cat ~/.ssh/id_ed25519.pub >> ~/.ssh/authorized_keys` and `chmod 600 ~/.ssh/authorized_keys`.
3. Trust this server's own key. Run `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` and note the `SHA256:` value. Run `ssh localhost`, check that SSH shows the same value, type `yes`, then type `exit`.
4. Run `connections join-invitation` with your invitation, then `auth` and `join`, as in [Join a workspace](#join-a-workspace).

When the invitation names this machine, by its name or one of its addresses, Crew plans the login `bob@localhost` with no jump host by itself. If it names the machine another way, add `--ssh-target localhost`. If you skipped step 2, connecting fails with `Couldn't sign in as bob on this machine: add your public SSH key to your own ~/.ssh/authorized_keys, then connect again.` If you skipped step 3, it fails with `localhost's host key isn't in your ~/.ssh/known_hosts yet.` and names the key file to add. A refusal that names `the jump host on this connection's route` and says `This computer is the workspace's server, so it needs no jump host` means a jump host came from the invitation. Join again with `--ssh-target localhost`, or clear **Jump hosts** in **Connection settings…**.

## Sign in and stay connected

- `status` or `connections list` prints one line per connection, such as `lab · bob@hpc.example.edu · Connected · Private (ucsf)`, plus the last error. The privacy is the one in force: a Public connection to a workspace that is Private for everyone reads `Private because lab is Private for everyone · your connection: Public`. `connections show` details one. A connection your host has not let in yet reads `Not joined yet` where a joined one reads `Connected`.
- When the workspace server has stopped saving changes, `status` and `connections show` add ``The workspace server has stopped saving changes. Reading still works.`` The host also gets what to run on the server, such as ``You host this workspace. Free space on hpc, then restart Crew there: biorouter-crew stop, then biorouter-crew start, each with this workspace's --state-dir.`` See [Server storage full or failing](administration.md#server-storage-full-or-failing).
- `auth` signs in inside your terminal and connects.
- `connect` connects without prompts. If the server wants a password or code, it fails with `The server wants your password or a verification code. Run biorouter crew auth to sign in.`
- `disconnect` closes the connection until you connect again.

The daemon reconnects by itself after a network drop, but not after `disconnect`, a password prompt, a refused key, a server key problem or removal. See [Connections and troubleshooting](connections-and-troubleshooting.md). A removed computer shows `Last error: This computer is no longer a member of lab.` While the daemon dials a broken connection again, a command answers `Reconnecting to lab. Nothing was sent; try again in a moment.` (code `crew_reconnecting`). Run it again a moment later. When the next try is further off, it says when, such as `Reconnecting to lab in about 60 seconds. Nothing was sent. Connect now to try at once.` Run `connect` to try at once. After a drop, the daemon tries after 20 seconds, then 1 minute and 3 minutes later, then every 5 minutes, for up to an hour. While the workspace is stopped on the server (`crew_broker_not_running`), it tries every 30 seconds.

A failed `connect` says what went wrong and what to run, then the code below on a `Code:` line and, when SSH gave a reason, its own words on a `Details:` line. `status` and `connections show` lead a connection's last error with the same advice.

| SSH failure code | What to do |
|---|---|
| `crew_ssh_auth_required` | The server wants a password or a verification code. Run `auth`. |
| `crew_ssh_key_refused` | The server refused this computer's SSH key and asks for nothing else, so `auth` cannot help. Check the login and key file in `connections show`, and ask IT which login and key to use. For a login on this machine, the sentence is ``Couldn't sign in as bob on this machine: add your public SSH key to your own ~/.ssh/authorized_keys, then connect again.`` Do that yourself, as step 2 of [Join from the workspace's own server](#join-from-the-workspaces-own-server) shows. |
| `crew_ssh_host_key_unknown` | Get the key fingerprint from IT, check it, and add the key to your known hosts file. For a login on this machine, the sentence is ``localhost's host key isn't in your ~/.ssh/known_hosts yet. Add this server's own key (from /etc/ssh/ssh_host_ed25519_key.pub) to it, then connect again.`` Do that yourself, as step 3 of [Join from the workspace's own server](#join-from-the-workspaces-own-server) shows. |
| `crew_ssh_host_key_changed` | Do not connect. Ask IT to confirm the change. |
| `crew_ssh_unreachable` | Check your network, or your VPN (the app that connects you to your institution's network). The daemon keeps trying. |
| `crew_bridge_missing` | Crew is missing from your server account. Run `setup`, which installs it and connects. Or install `~/.local/bin/biorouter-crew` yourself, or ask IT ([Install Crew in your server account](joining-a-workspace.md#install-crew-in-your-server-account)). |
| `crew_workspace_deleted` | The host deleted the workspace on the server. The sentence is ``The host deleted this workspace on the server. Remove it from this computer with biorouter crew connections remove.`` Run `connections remove`, which skips the host warning for a deleted workspace. |
| `crew_broker_not_running` | SSH worked, but the workspace is not running on the server, for example after the server restarted. If you host it, start it as [After the server restarts](hosting-a-workspace.md#after-the-server-restarts) shows. Otherwise, ask your host. Then run `connect`. |
| `crew_workspace_identity_mismatch` | Stop. The server answered for a different workspace. Ask your host. |
| `crew_ssh_failed` | SSH or Crew on the server failed in another way. If the server restarted, ask your host to start the workspace again as in [Server commands](#server-commands), then run `connect`. |

`auth` says `Signed in, but Crew couldn't start on the server` when that copy is missing, and `terminal could not be attached` when another window is already signing in to this connection. Run `setup` then: it uses the same sign-in.

A connection whose workspace the host deleted reads that way in `status` and `connections show`, and every command that needs the workspace answers `crew_workspace_deleted` rather than suggesting `connect`.

## Change or remove a saved connection

`connections remove` asks you to type the connection's name first. In a script, add `--confirm NAME`. It disconnects, ends every chat's access through that connection, and deletes this computer's device key for the workspace. When the connection is up, it first asks the workspace to revoke that access. It never connects to do so, so a workspace that was offline keeps honoring that access on the server until it expires, an hour at most. Your messages stay on the server, and you stay a member, so your old invitation does not bring the workspace back. To use it here again, follow [Add this computer to your existing account](joining-a-workspace.md#add-this-computer-to-your-existing-account), or from a terminal:

1. Ask the host to run `enroll invite @bob --add-device` and send you the new invitation.
2. Run `connections join-invitation` with it, then `auth` and `join`, as in [Join a workspace](#join-a-workspace).
3. Send the new code to the host. When the host enters it, `join` prints `You're in lab.`

For a connection whose workspace the host deleted, `connections remove` asks only for the name and does not warn about host controls. If you host the workspace and no other computer of yours can act as its host, `connections remove` refuses and names the `enroll invite @alice --add-device` command that adds one. `--give-up-host-controls` removes it anyway, and then nothing can restore the host controls. Read [Limits of the host role](hosting-a-workspace.md#limits-of-the-host-role) first.

If the workspace lists another computer enrolled as you, `connections remove` goes ahead once you confirm, but first it lists those computers by fingerprint and the date each was added. The host controls continue only if one of them still has the workspace saved. A computer stays on that list after its connection is removed there, so the list alone does not prove it can still act as host. Run `connections list` on that computer before you go on. With `--confirm` or in a script, the list is printed on stderr.

`connections save FILE` adds a connection from a JSON description, and `connections update FILE` replaces the selected one. The fields are listed in [Save a connection from a descriptor](design/cli-guide.md#save-a-connection-from-a-descriptor). The output of `connections show` is not valid input.

### Agent access

`connections set-access off`, `folder` or `full` sets what agents may do on the server through the selected connection ([What your agent may do on the server](agents-and-chat-access.md#what-your-agent-may-do-on-the-server)).

- `off` removes the work folder. It prints ``Agents can't work on hpc through lab now.``
- `folder` confines agents to one folder. `--folder PATH` names it, as a full path or `~/` once this computer has connected, and the connection's own folder is the default. On a server whose kernel cannot confine commands, it says that commands there need full access.
- `full` lets agents run commands as your account, starting in that folder. It asks first, naming what full access allows; add `--yes` where there is no terminal to ask in. It is refused when the Crew in your server account is too old for it: run `setup --replace-existing` first.

Without a folder, `folder` and `full` exit with `2` and code `crew_folder_required`. A change of level ends every chat's and task's access through the connection, and the command says so.

## Set up Crew on a server

`setup` installs or updates Crew's server program, `~/.local/bin/biorouter-crew`, in your own account on a server, over SSH, as the desktop app does ([How Biorouter installs Crew](administration.md#how-biorouter-installs-crew)).

- `setup` with a saved connection (the selected one, or `--connection NAME`) installs Crew if it is missing, then connects. With no work folder on the connection yet, it also gives agents full access, starting in a new `~/crew-work/WORKSPACE` folder, as joining in the desktop app does. `--agent-access off`, `folder` or `full` chooses the level instead, and a connection that already has a folder keeps its level unless you name one. `off` changes nothing and makes no folder.
- `setup --server LOGIN` installs Crew for a server you have no connection to yet, such as `bob@hpc.example.edu`, and does nothing more. `--port`, `--identity-file` and `--proxy-jump` set the route.
- `--replace-existing` replaces an older copy, or a file Biorouter did not put there, keeping the old one as `biorouter-crew.previous`. Without it, setup keeps an older copy that runs. A newer copy is never replaced.
- `setup --check`, or `doctor` with the same options, checks the server and changes nothing. It prints the server's system and glibc, whether Crew is installed and which version, and, when the kernel cannot confine commands to a folder, ``Agents can run commands there only with full access: its kernel can't confine them to a folder.``

`setup` prints one line per step as it starts, such as checking the server, signing in and installing, then a summary such as `Installed Crew 1.93.0 in your account on hpc.` or `Crew 1.93.0 was already installed in your account on hpc.`. A same-version copy with other bytes is replaced, and the summary names both copies. If the server asks for a password or a code, it asks in the terminal, and that one sign-in covers the rest. Ctrl+C stops the setup, and running the same command again finishes it. With `--output-format stream-json`, each step is one JSON value.

### Setup refusals

A failed setup prints its sentence and exits with `1`. In JSON its `code` is one of these, and `detail` holds what the server printed, bounded and with control characters removed.

| Code | What it means |
|---|---|
| `crew_setup_busy` | Two setups are already running on this computer. Wait, then try again. |
| `crew_setup_not_found` | The daemon lost track of the setup, for example because it restarted. Run it again. |
| `crew_setup_cancelled` | The setup was stopped. Running it again finishes it. |
| `crew_setup_timed_out` | The server took too long to answer. |
| `crew_setup_sign_in_expired` | The setup waited 10 minutes for a sign-in. |
| `crew_setup_not_waiting_for_sign_in` | A sign-in arrived for a setup that no longer waits for one. |
| `crew_setup_server_unreadable` | The server's answer could not be read, so nothing changed. |
| `crew_setup_start_failed` | Crew is installed, but it did not start the workspace. `detail` holds what Crew printed. |
| `crew_setup_changed` | The connection's login or route changed while the setup ran. Run it again. |
| `crew_server_probe_unsupported` | The Crew there is too old to describe the server. Run `setup --replace-existing`. |
| `crew_install_artifact_unavailable` | This Biorouter carries no Crew server program. Update Biorouter, or install Crew yourself. |
| `crew_install_unsupported_platform` | The server is not x86_64 Linux, or the program there is not built for it. |
| `crew_install_dependency_missing` | The server lacks a system library Crew needs. Ask IT. |
| `crew_install_incompatible` | The Crew in your account does not work with this Biorouter. Run `setup --replace-existing`. |
| `crew_install_unexpected_target` | `~/.local/bin/biorouter-crew` is a link, a folder or another account's file, which setup never replaces. |
| `crew_install_checksum_tool_missing` | The server has none of `sha256sum`, `shasum` or `openssl`. Ask IT. |
| `crew_install_no_space` | There is not enough free space in your home folder. |
| `crew_install_permission_denied` | Your account cannot write to `~/.local/bin`. |
| `crew_install_busy` | Another Biorouter is installing Crew in your account right now. |
| `crew_install_transfer_incomplete` | The connection ended before the whole program arrived. Nothing was installed. |
| `crew_install_checksum_mismatch` | The program that arrived does not match the one sent. Nothing was installed. |
| `crew_install_outcome_unknown` | Biorouter cannot tell whether the install finished. Run setup again to check. |
| `crew_host_storage_unsafe` | A host's folders above `~/.local/share/biorouter-crew` can be changed by other accounts. The sentence names them and the `chmod go-w` command, and `detail` lists them. Run `host --fix-permissions` to let setup fix them ([Storage permissions](administration.md#storage-permissions)). |
| `crew_host_storage_unsupported` | The home folder is on network storage, where Crew cannot keep a workspace. |
| `crew_host_home_unsafe` | Other accounts can change your home folder. Run `chmod go-w ~` on the server. |
| `crew_host_pidfd_unavailable` | The server's Linux is too old to host a workspace. |
| `crew_host_machine_id_unavailable` | The server has no readable `/etc/machine-id` ([Machine identity](administration.md#machine-identity)). |
| `crew_host_setup_unknown`, `crew_host_setup_used` | The hosting preparation is unknown or already used. Run `host` again. |
| `crew_host_start_busy`, `crew_host_start_cancelled`, `crew_host_start_timed_out` | Starting the workspace was already running, was stopped, or took too long. |

## Host a workspace

`host` does the whole job in one command: it installs Crew in your account on the server, starts the workspace, saves and connects it here, creates it with this computer as its first admin device, sets its privacy and institution, makes it open to every member, saves a [workspace context](agents-and-chat-access.md#the-workspace-context) drafted from the server, and gives your agent full access there.

```bash
biorouter crew host --name lab --server alice@hpc.example.edu --institution ucsf
```

- `--name` is 1 to 40 lowercase letters, numbers and dashes, starting and ending with a letter or number. `--server` is your SSH login or an alias from your SSH settings.
- `--mode private` (the default) needs `--institution ID` (code `crew_institution_required`); `--mode public` allows Public.
- `--port`, `--identity-file` and `--proxy-jump` set the route.
- It lists what it will do and asks `Go ahead?`. Add `--yes` where there is no terminal to ask in.
- If other accounts can change folders above where Crew keeps the workspace, it stops before starting, with `crew_host_storage_unsafe` and the folders. Run it again with `--fix-permissions` to let it run `chmod go-w` on exactly those folders.

It ends with lines such as `lab is Private (ucsf) and open: every member can manage teams, channels and people.`, the work folder your agent starts in, the context's revision, and `Next: invite people with biorouter crew --connection lab enroll invite @USERNAME`. A server whose Crew is older than open workspaces keeps the workspace managed, and says so. If a step after the start fails, the error names what is done and the command that finishes it.

In the desktop app, choose **Host a new workspace**, then **Set up and start Crew**, as [Hosting a workspace](hosting-a-workspace.md) describes. To do each step yourself instead:

1. On your computer, run `biorouter crew connections prepare --show-ids` (`enroll prepare` is the same command). Note the public key and the `Preparation ID:`, which only `--show-ids` shows.
2. On the server, in your own SSH session, run the command below. The name is 1 to 40 lowercase letters, numbers and dashes, and starts and ends with a letter or number. Copy the JSON it prints into a file on your computer, such as `lab-start.txt`.

   ```bash
   "$HOME/.local/bin/biorouter-crew" start --name lab --bootstrap-key 'PUBLIC_KEY_HEX'
   ```

3. On your computer, run `biorouter crew connections join-invitation ./lab-start.txt --preparation-id 'PREPARATION_ID' --ssh-target alice@hpc.example.edu --institution ucsf`. It ends with the next two commands, `auth` and `workspace bootstrap`.
4. Run `biorouter crew auth`, then `biorouter crew workspace bootstrap`, which works once, with the key you gave `start`. It prints `Signed in to lab as @alice.` Members see your display name after you run `profile set 'Alice Chen'`.
5. Run `biorouter crew privacy set-workspace private --institution ucsf`. The institution can never change, so check it before anyone runs an agent. The command prints a line such as `Workspace privacy: Private for everyone · institution ucsf · policy epoch 2`.

`workspace show` lists its privacy, people, teams, channels, invitations and agent grants. `workspace rename NAME` (host) keeps the history and grants.

### Manage the workspace

| Command | Notes |
|---|---|
| `workspace governance open` or `managed` | Host only (code `crew_host_required`). Open lets every member manage teams, channels and people and let people in; managed keeps it to channel owners, team creators and you ([Roles and permissions](teams-channels-and-people.md#roles-and-permissions)). |
| `workspace context show` | Prints the [workspace context](agents-and-chat-access.md#the-workspace-context), with its revision and who saved it. |
| `workspace context set FILE` | Replaces it with a Markdown file of up to 64 KB, or `-` for standard input. Every member in an open workspace, the host in a managed one. |
| `workspace context edit` | Opens it in `$VISUAL` or `$EDITOR`, then saves it. If someone saved a newer version meanwhile, it says so and keeps your text. |
| `workspace context draft [--save]` | Drafts a context from a fixed check of the server and prints it. `--save` also saves it. |
| `workspace delete` | Host only. Deletes the workspace on the server for everyone, then removes it from this computer. It asks you to type the workspace's name; in a script, add `--confirm NAME`. A member gets the command that removes it from their own computer instead, and nothing is sent. See [Delete the workspace](hosting-a-workspace.md#delete-the-workspace). |

A context saved against a version someone else replaced is refused with ``Someone changed the workspace context since you read it, so yours wasn't saved. Read it again with biorouter crew workspace context show, then make your change.``

## Let people into the workspace

The host runs `enroll` commands, and in an open workspace every member can.

1. Run `biorouter crew enroll invite @bob`. Crew checks that this server account exists and prints an invitation message. Send the whole message to Bob. It holds no secret.
2. When Bob sends his code, run `biorouter crew enroll approve @bob 7QK2-M9XA-3JTP-WZ4D`. Case, spaces and dashes do not matter. It prints `Code saved. @bob joins when their computer confirms the same code.`
3. Bob is in only when his `join` prints `You're in lab.`

`enroll pending` lists who is waiting. If you mistyped a code, Bob's `join` reports a mismatch and `enroll pending` warns under his row. Run `enroll approve` again with `--replace`. Approve only a code the person sent you.

- `connections invitation --for @bob` prints the invitation again. `enroll invite @bob --add-device` lets a member add another computer. `enroll cancel @bob` withdraws an invitation.
- An invitation lasts 24 hours. Inviting again replaces it. At most 100 people can wait at once.
- `enroll revoke @bob` (host only) removes a member with all their computers, and asks you to type `@bob` again. In a script, add `--confirm @bob`. It ends every agent grant in the workspace. A removed person invited again starts with no teams or channels.

### Tokens for older servers

A server whose `biorouter-crew` cannot join by code needs the older token path. Its commands (`enroll invite --uid UID --public-key HEX`, then `enroll accept`) are hidden from `--help`, and a token works once, within one hour. See [Enroll with a token](design/cli-guide.md#enroll-with-a-token-older-versions), and update the server when you can.

## Manage your profile, teams and channels

| Command | Notes |
|---|---|
| `profile show` | Your name and enrolled computers. |
| `profile set 'Bob Lee' --avatar 'BL'` | Up to 12 characters of initials or emoji. Leaving out `--avatar` removes them. |
| `teams list`, `teams create NAME` | A new team gets a `general` channel. In an open workspace, it gets every member too. |
| `teams rename TEAM NAME` | Team creator, or in an open workspace any member of the team. The team's handle, the name commands take, changes with it, and the old one stops working. |
| `teams join TEAM` | Open workspace only. Joins the team and its `#general`. |
| `teams delete TEAM` | Deletes the team for everyone, with all its channels and their messages and files. Team creator, the host when the creator left, or in an open workspace any member of the team. Asks first; add `--yes` in a script. |
| `channels list [--team TEAM]` | Shows classification, owner and unread count, grouped by team with each team's `#general` first. In JSON, each channel has `unread`. |
| `channels create NAME --team TEAM` | `--classification restricted` (default) or `public-safe` (JSON writes `public_safe`). |
| `channels rename`, `channels archive` | Owner, or in an open workspace any member of the channel. Archiving asks first, needs `--yes` in a script and cannot be undone. An archived channel stays readable and takes no posts. |
| `channels join CHANNEL` | Open workspace only. Joins a channel of one of your teams. |
| `channels delete CHANNEL` | Deletes the channel for everyone, with its messages and files. Owner, the host when the owner left, or in an open workspace any member of the channel. A team's `#general` goes only with its team. Asks first; add `--yes` in a script. |
| `channels mark-read CHANNEL` | Marks the channel read. |

A display name is a label only. It cannot contain `@` or `#` or match a username. Only private AI models can read a Restricted channel. See [Privacy and security](privacy-and-security.md). Name rules are in [Teams, channels and people](teams-channels-and-people.md). After ten names that were already taken within ten minutes, you get `Too many name attempts. Try again later.` Wait ten minutes. On a server whose Crew is older than open workspaces, `join` and `delete` say ``This workspace's server can't …`` and that `setup` updates it.

## Add people to teams and channels

- `members` lists everyone, host first, marking `you`, `host`, `online`, `former member` and `account no longer valid`. In JSON, each person has `is_you`, `is_host` and `is_online`. Online means the person's computer is connected to the workspace now, or made a request in the last three minutes. A server with an older `biorouter-crew` reports no one as online.
- `members add @bob --team TEAM` (team owner or host, or in an open workspace any member of the team) adds Bob to the team and its `#general`, and to each `--channel` of that team. Bob does not need to accept. It prints a line such as `Added "Bob Lee" (@bob) to Analysis Lab. They can now see #general.` Bob can read everything already posted in those channels, files included.
- Without `--team`, each `--channel` must be one you own, or in an open workspace one you are in, in a team Bob is already in. The host may name any channel there. Each channel is added separately, so before adding any, the command checks them all against your view of the workspace and adds nothing if one would be refused. If one still fails, the error names the channels already added.

| Task | Command |
|---|---|
| Invite someone, who then accepts (team creator or channel owner) | `invites create @bob --team TEAM` or `--channel CHANNEL` |
| Accept an invitation | `invites list`, then `invites accept [TEAM or TEAM/CHANNEL]` |
| Remove someone from a channel (channel owner) | `remove-member CHANNEL @bob`, which asks first and needs `--yes` in a script |
| Hand over a channel (owner, then Carol) | `ownership offer CHANNEL @carol`, then `ownership accept CHANNEL` |

A channel invitation needs the person in its team first. `invites accept` exits with `1` when nothing is pending. Add `--former` to `remove-member` for someone who left the workspace. After a handover, the previous owner leaves the channel.

## Post and read messages

- `send methods --text 'Ready.'` posts under your name. `--input FILE` reads the text from a file, or `-` for standard input. `--attachment ID` and `--reference ID` add files and can repeat.
- A message is at most 64 KB. A longer one is refused before anything is sent, with ``Messages can be up to 64 KB. Save the text to a file and share it with biorouter crew files upload.`` (exit `2`, code `crew_message_too_long`).
- `history methods --latest` shows the newest 100 messages. Without `--latest`, it starts from the oldest. `--limit` takes 1 to 200. With `--show-ids`, it ends with a `Cursor:` line to pass to `--before` or `--after`.
- `search methods 'qPCR'` finds messages in one channel, in any case.
- `watch methods` shows the newest messages, up to 200, then prints new ones as they arrive. `--new-only` prints only messages posted from now on, `--from-start` replays the channel from its oldest message, and `--after CURSOR` starts after a cursor. Ctrl+C stops it without cancelling tasks. If the daemon ends it, for example because you lost access, it prints why and exits with `1`.
- `history`, `search` and `watch` name each attached file on its own line, such as `Attachment: counts.csv (55 KB)`, and `--show-ids` adds its ID. In JSON, `attachment_details` maps each ID to its `name`, `size` and `media_type`.

Reading with `history`, `search` or `watch` leaves the channel unread, in the desktop app too. Run `channels mark-read methods` when you have read it. The command line sends no notifications, so keep a `watch` running to follow a channel from a terminal. The desktop app notifies you, as [Unread messages](messages-and-files.md#unread-messages) describes. Message limits are in [Messages and files](messages-and-files.md).

## Share files and server paths

Uploading does not post. You upload, then attach:

1. `biorouter crew --show-ids files upload methods ./counts.csv` prints the transfer ID.
2. `files watch TRANSFER_ID` follows it to `Transfer Ready.`
3. `--show-ids files status TRANSFER_ID` shows the `Attachment ID:`.
4. `send methods --attachment ATTACHMENT_ID` posts it, and prints `Posted to #methods.`

To list a channel's files, run `files list methods`. It prints the files shared there, newest first, each with its name, size, who shared it and when, and its attachment ID. `--limit` takes 1 to 200 (default 50).

To download:

1. Run `files list methods`, or `history methods --latest --show-ids`, where each file shows as `Attachment: counts.csv (55 KB)` with its ID. `files show ID` shows one file's name, size, type and channel.
2. Run `biorouter crew --show-ids files download ID --output ./counts.csv`. It returns as soon as the transfer starts and prints the transfer ID. Until the transfer finishes, the folder holds only a partial file named `.biorouter-crew-TRANSFER_ID.part`.
3. Run `files watch TRANSFER_ID`. The file is there when it prints `Transfer Saved.`

`--output` names the file, not a folder. Add `--overwrite` to replace an existing file.

- `files pending` lists transfers. `files pause ID` pauses one, and `files resume ID FILE` resumes it with the original file.
- `files forget ID` removes this computer's record. For an unfinished download, add `--file PATH` to delete the partial file. To cancel an unfinished upload, run `files pause ID`, wait until `files status ID` reads `Paused`, then run `files forget ID`. The part already sent stays on the server for up to a day and counts toward the workspace's file space, at its full size, until then.
- `files watch` exits with `0` however the transfer ends, so read the last line, such as `Transfer Ready.` for an upload or `Transfer Saved.` for a download. The state is `Ready`, `Saved`, `Paused`, `Failed` or `Not confirmed`. `Paused` can be resumed; it follows `files pause`, a dropped connection, a locked vault, two other transfers running, or a workspace server that could not save the file, whose own sentence then says so. `files status` prints the reason and the `files resume` command to run. `Failed` means the workspace refused the transfer, so resuming cannot help. Ctrl+C leaves the transfer running.

Crew refuses a download into a folder that is not yours or that other accounts can change, into a credential or settings folder, through a folder that is a link, or over a program, and a file name in your home that starts with a dot. Each refusal says what to change. A path through a link says so and names the real folder, such as ``/Users/bob/shared goes through a link. Choose the folder it points to, /Volumes/lab/shared.`` On a Mac, `--output /tmp/counts.csv` goes through the link `/tmp` to `/private/tmp`, which other accounts can change, so it is refused with ``Choose a folder owned by your account that other accounts can't change.`` It refuses to upload files that look like credential stores: it checks each file's name, and reads its first and last 64 KB. Size limits are in [Administration](administration.md).

To share a server path without uploading it, run `--show-ids files reference methods /project/results --label 'Results'`, then `send methods --reference ID`. Crew does not check the path. `files show-reference ID` shows one.

## Run an agent task

A task runs your own AI agent on a prompt, lets it read the channel, and posts its result there.

```bash
biorouter crew ask methods Sum the plate counts in plate_counts.csv
biorouter crew tasks start methods --input ./prompt.txt --allow-posting
```

- `ask CHANNEL TEXT…` is the short form: it joins the words into the task and starts it on your model for Crew tasks, allowed to post its result in the channel.
- `tasks start` gives the prompt with `--text` or `--input`, and `--allow-posting` is required. `--provider` and `--model`, together, choose the model; without them, the task starts on your model for Crew tasks, else your default model for new chats, and with neither it exits with `2` (code `crew_task_model_required`).
- `--provider` is the name in your Biorouter configuration.
- Each `--context-channel CHANNEL` adds a channel to read, up to 16, or up to 99 in an open workspace on a current server.
- The task's access lasts one hour, and the task stops after 60 model turns, 200 tool calls or 55 minutes.
- What the agent may do on the server follows the connection's [agent access](#agent-access).
- In a Private workspace, the model must be approved for the workspace's institution, or run locally. See [Agents and chat access](agents-and-chat-access.md).
- The result ends with a line naming the shared files and the remote work folder files it read, or saying it read none.

The model for Crew tasks is the one the desktop's **Settings** > **Models** > **Crew tasks** row sets, kept for every workspace on this computer:

- `defaults show` prints it and where it comes from, such as `Crew tasks start on gpt-5.5 (versa_azure).` or `Crew tasks use your default model for new chats: …`.
- `defaults set --provider PROVIDER --model MODEL` sets it, and prints ``Crew tasks now start on gpt-5.5 (versa_azure), unless a task names another model.``
- `defaults clear` goes back to your default model for new chats.

The workspace still checks each task's model against its privacy and institution.

`tasks list` shows each task, the channel it posts to and its chat session, and `--show-ids` adds the task IDs that `tasks show`, `tasks watch` and `tasks cancel` take. `tasks watch ID` follows one and exits with `0` at every end state, so read the last: `Done`, `Couldn't finish`, `Stopped`, `Interrupted`, `Outcome unknown` or `Stop not confirmed`. Ctrl+C stops watching, not the task.

`tasks cancel ID` stops the task on your computer and revokes its access, but cannot confirm that a process on the server ended. If the workspace cannot be reached, it exits with `1`. Run `tasks cancel` again to retry.

## Give a chat access to Crew

The desktop app does this with `/crew`, as [Agents and chat access](agents-and-chat-access.md) explains. From a terminal, find the chat's session ID with `biorouter session list`. The chat must be open in the shared daemon, in the desktop app or a terminal chat, and idle.

```bash
biorouter crew grants grant SESSION_ID methods --context-channel analysis-lab/raw-data
```

The first channel is where the chat posts. Each `--context-channel` adds one to read, up to 20 channels in all. `grants list` shows each chat and task with access, its state and time left. `context SESSION_ID` starts with the grant's channels, such as `Access: #methods · also reads #raw-data`, then lists the messages the chat can read, oldest first. Each post the chat makes ends with a line from your daemon naming the shared files and the remote work folder files it read since its last post, or saying it read none.

Some workspace changes end every grant and task in the workspace. `grants list` then shows `Ended: Crew settings changed`, and `context` says ``This chat's Crew access ended because Crew settings changed.`` (code `crew_grant_ended`), followed by the `grants grant` command that grants it again. Run that command. After a change to your own connection's settings, `grants list` can go on showing the grant as `Active` until its time runs out, but `context` and the chat refuse it all the same. [Why settings changes end access](agents-and-chat-access.md#why-settings-changes-end-access) lists the changes.

`grants revoke SESSION_ID` exits with `0` only when the workspace confirms. After `Stopped on this device ...` (exit `1`), the chat already cannot use Crew, and Biorouter confirms later by itself. `Not revoked` means the chat still has access, so retry. A revoke does not prove that a command already running on the server stopped.

To use a terminal chat:

1. Run `biorouter session --shared-daemon --no-start --create-only --provider PROVIDER --model MODEL`, with a model the workspace accepts ([Agents and chat access](agents-and-chat-access.md#before-you-start)). A new chat needs both options. It prints the chat's session ID and a line such as `Chat 20260927_1 is ready (versa_azure/gpt-5.5).`
2. Run `grants grant SESSION_ID methods`. `grants list` then shows the chat as `Active`. The grant fixes the chat's model.
3. Open the chat with `biorouter session --shared-daemon --no-start --resume --session-id SESSION_ID`, or send one prompt with `biorouter run` and the same options plus `--text`. The chat opens in your terminal, or `biorouter run` prints its reply.

Without `--shared-daemon`, the chat cannot use a grant.

## Privacy settings

`privacy show` prints the privacy in force first, then the privacy, institution and policy epoch of your connection and of the workspace, and each channel's classification. While you are disconnected, it says the workspace's setting cannot be checked. In JSON, `effective_mode` is the privacy in force, or `null` when it cannot be checked.

- `privacy set-personal private --institution ID` or `public` changes how this computer treats the workspace.
- `privacy set-workspace private` or `public` (host) changes the workspace's privacy. `--institution ID` confirms its institution, which never changes.

`set-personal public` and `set-workspace public` ask you to type the workspace's name first, as the desktop app does. In a script, add `--confirm WORKSPACE`. Going back to private asks nothing. In a workspace that is Private for everyone, `set-personal public` saves your choice but says that nothing changes until the host allows Public.

Saving your connection's privacy or institution ends your chats' access through it. A connection that was connected reconnects at once, and one that was disconnected stays so.

A Private connection's institution must be the workspace's, once the workspace has one, since the host fixes it for good. Another one is refused and nothing is saved, such as ``This connection is for stanford, but lab belongs to ucsf. Use ucsf here.`` (code `crew_institution_mismatch`, with `connection_institution` and `workspace_institution` in JSON). An institution ID is 1 to 64 lowercase letters, digits, `_` or `-`. Changing the workspace's privacy ends every agent grant. Switching to Public never exposes earlier Private content. See [Privacy and security](privacy-and-security.md).

To stop a script when the policy changed after it checked, pass the epochs from `privacy show` to `--expected-policy-epoch` and `--expected-workspace-policy-epoch`. For posts and file transfers, use `--expected-mode private`.

## Retry after an uncertain result

If a change may have reached the workspace but the answer was lost, the error says so, such as `Crew couldn't confirm whether this reached lab. Check the channel, then retry with the same request ID.`, and ends with `Retry safely with --request-id ID`. In JSON its code is `crew_outcome_unknown`. Check the channel first, then run the same command again with that option. The workspace does not apply the change twice. A server whose disk failed while saving the change says `…so it may not have been saved.` and gets the same line.

A clear refusal never shows this line, and neither does a request that was never sent, such as one answered `crew_not_sent` or `crew_reconnecting`. Run that one again as it was.

An ID is 1 to 128 letters, digits, `_` or `-`. Use a new ID for a different change. To continue a transfer, use `files resume`, not a new upload.

## Recover after a restart

After your computer or the daemon restarts, check in this order:

1. Run `daemon status`. Any command starts a missing daemon.
2. Run `credentials unlock` if you use the vault.
3. Run `status`. Run `auth` or `connect` for a `Disconnected` connection.
4. Run `files pending`, `tasks list` and `grants list`.
5. For a task in `Interrupted`, `Outcome unknown` or `Stop not confirmed`, read its channel before you start it again. Repeat `tasks cancel` if the message asks. A task that was stopping when the daemon stopped reads `Stop not confirmed`, and Biorouter confirms it with the workspace by itself the next time the connection is up.

If a command reports a changed key or a daemon it cannot verify, find out why before you connect. Do not replace keys or files to silence it.

## Workspace refusals

Workspace refusals are plain sentences that name the fix, with the code in JSON as `broker_code`. A refusal for a role names the role: `Only the workspace host can do this.`, `Only #methods's owner can do this.`, `Only the team's owner can do this.` or `Only the team's creator can do this.`. A post in an archived channel gets `#methods is archived, so it's read-only.`. These need a note:

- `You're not in that channel.` You were removed from it, or the ID is wrong.
- `The workspace didn't allow this.` The workspace refused for a role it did not name. Check that you own the channel or team, or host the workspace.
- `Account enrollment changed.` Your server account was renamed or replaced. Ask the host to remove you and invite you again.
- A sentence that starts `The workspace server is out of disk space` or `The workspace server could not` means the server cannot save changes. Reading still works. If you host the workspace, a second line says what to run; see [Server storage full or failing](administration.md#server-storage-full-or-failing).
- `The workspace refused this request.` Run the command with `--output-format json` and send the `broker_code` to your host.
- `This workspace was renamed since you chose it` (`name_changed`): a channel, team or workspace you are deleting was renamed in the meantime. Check its name, then try again.

Your own daemon refuses some requests before they reach the workspace, each with its own code in JSON:

| Code | What it means |
|---|---|
| `crew_mode_mismatch` | `--expected-mode` named the other privacy mode. The sentence names both, and nothing was sent. |
| `crew_institution_mismatch` | The model, your connection and the workspace belong to different institutions. The sentence names them. |
| `crew_public_model_refused` | A public model may not read this workspace or channel. Choose a private model. |
| `crew_channel_not_in_workspace` | The channel ID is not a channel of this workspace. |
| `crew_model_fixed` | The chat's model is fixed by its Crew access. Start a new chat for another model. |
| `crew_reconnecting`, `crew_not_sent` | Nothing was sent. Run the command again once Crew reconnects. |
| `crew_outcome_unknown` | The request may have reached the workspace. See [Retry after an uncertain result](#retry-after-an-uncertain-result). |
| `crew_credential_store_unavailable`, `crew_credential_store_refused` | Crew cannot use this computer's keyring. See [Keep device keys in an encrypted vault](#keep-device-keys-in-an-encrypted-vault). |
| `crew_not_connected` | The connection is disconnected, and Crew is not dialling it again. The command says ``lab is disconnected. Run biorouter crew connect, then try again.`` Run `connect`, or `auth` if the server asks for a password or code. Nothing was sent. |
| `crew_grant_ended` | The chat's Crew access ended. In JSON, `reason` is `settings_changed` or `ended`. The sentence names the `grants grant` command that grants it again. |
| `crew_registry_unreadable` | Crew's saved connections on this computer (`connections.json`) were saved by a newer Biorouter, or are damaged, so nothing was changed. Update Biorouter, or restore the file from a backup. In JSON, `detail` holds what could not be read, for support. |
| `crew_file_name_invisible` | The file's name has an invisible or formatting character. Rename the file, then share it again. |
| `crew_destination_exists` | The file that `files download --output` names already exists. Add `--overwrite` to replace it, or choose another name. |
| `crew_workspace_deleted` | The host deleted the workspace. Run `connections remove`. |

### Other daemon codes

The daemon answers these codes too. A command prints the sentence beside each, and JSON carries the code, so a script can match it.

| Code | What it means |
|---|---|
| `crew_user_action_required`, `crew_human_authority_unavailable` | The request carried no proof that a person asked, or the daemon holds no key to check one. Crew does not work under `biorouter serve` for this reason. |
| `crew_request_invalid`, `crew_request_refused` | The daemon could not read the request, or refused it; `detail` or `broker_code` says why. |
| `crew_connection_not_found`, `crew_connection_exists` | No saved connection has that ID, or this computer already has the workspace with other settings. |
| `crew_membership_ended` | The workspace no longer admits this computer or account. |
| `crew_handoff_failed` | SSH signed in, but Crew did not start behind it. With `bridge_missing: true`, Crew is missing from the account: run `setup`. |
| `crew_invalid_selector` | A name that is not a valid username, team, channel or connection. |
| `crew_invitation_invalid`, `crew_invitation_conflict` | The pasted invitation cannot be read (`reason` says why), or this computer pins a different identity for that workspace. |
| `crew_join_not_approved`, `crew_join_code_mismatch`, `crew_join_not_invited`, `crew_join_expired` | Not let in yet, a different code, no invitation, or an expired one. See [Join a workspace](#join-a-workspace). |
| `crew_join_replaced`, `crew_join_account_changed` | The invitation was replaced, or the server account (or the person who invited you) changed. Ask for a new invitation. |
| `crew_join_unsupported` | The server cannot join by code. See [Tokens for older servers](#tokens-for-older-servers). |
| `crew_join_device_conflict`, `crew_join_identity_conflict`, `crew_join_refused` | This computer's key is already in the workspace, another member has your username, or the workspace refused. Tell your host. |
| `crew_idempotency_conflict` | The `--request-id` belongs to a different request. Use a new ID. |
| `crew_start_outcome_unknown` | A task start was admitted but not finished. Check the task before starting another. |
| `crew_cancel_persistence_failed` | A task's stop could not be saved. Check the task, then cancel again. |
| `crew_session_unavailable` | The chat is not open in the shared daemon, or is out of reach. |
| `crew_grant_not_found`, `crew_grant_other_connection`, `crew_grant_replaced` | The chat holds no grant here, its grant is on another connection, or it was granted again while you revoked it. |
| `crew_revocation_unconfirmed`, `crew_revocation_not_saved` | The workspace has not confirmed the revoke yet, or the stop could not be saved here. Retry. |
| `crew_typed_run_required` | Task and grant methods go through `tasks` and `grants`, not a raw request. |
| `crew_profile_refused` | The key store's state could not be read. |
| `crew_transfer_refused` | The workspace refused the upload or download. |
| `crew_file_is_credential`, `crew_file_is_program`, `crew_file_name_hidden`, `crew_folder_shared`, `crew_destination_is_folder` | A file or download place Crew refuses: a credential, a program it would replace, a dot name in your home, a folder other accounts can change, or a folder where a file name belongs. See [Share files and server paths](#share-files-and-server-paths). |
| `crew_host_start_not_found` | No such start of a workspace on this computer. |

## Server commands

The server program `biorouter-crew` runs in the host's account. Members never run it, because their daemons start it over SSH. Its commands, `start`, `status`, `stop`, `delete`, `probe` and `doctor`, are in [Administration](administration.md). `status --name` and `stop --name` take the workspace's original name.

Nothing starts the workspace when the server boots. The host starts it again as [After the server restarts](hosting-a-workspace.md#after-the-server-restarts) shows.

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
- [Crew CLI guide for developers](design/cli-guide.md): design notes and description fields.
