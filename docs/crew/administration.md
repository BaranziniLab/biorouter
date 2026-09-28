# Crew administration

> **What this is.** What the Linux server needs for Crew, and how to install, start, upgrade, back up and troubleshoot a workspace.
> **Status:** Current. Describes Biorouter 1.91.2.
> **Audience:** Lab managers who host a workspace, and the IT staff who support its server.

A Crew workspace holds a lab's channels, shared files and AI agents. The host runs `biorouter-crew` on a Linux server as their own account. This broker keeps the workspace in files owned by that account, listens only on a Unix socket under `/tmp`, and learns each caller's account from the kernel. When a member connects, Biorouter starts the same program over SSH in their account, as a bridge to that socket. Crew needs no administrator rights, system service or open network port. For the app, see [Hosting a workspace](hosting-a-workspace.md).

Examples: host `@alice`, member `@bob`, server `hpc.example.edu`, workspace `chen-lab`. Angle brackets, such as `<version>`, mark a value you fill in.

## Server requirements

Crew needs Linux on x86_64 with glibc 2.31 or newer, and was checked on Debian 11 and Ubuntu 24.04. No ARM build is qualified. Acceptance testing did not cover Windows or Linux desktops, SSH passwords with verification codes, jump hosts, changed host keys, disk faults, power loss or restoring a copy.

### Check a server before you host

Run these on the server as the hosting account. They change nothing, and the last works only on x86_64.

```bash
ldd --version | head -n 1
uname -srm
df -P -T "$HOME" | tail -n 1
stat -c '%a %U %n' "$HOME" /etc/machine-id
python3 -c 'import ctypes,json,os; libc=ctypes.CDLL(None,use_errno=True); libc.syscall.restype=ctypes.c_long; a=libc.syscall(444,None,0,1); ae=ctypes.get_errno() if a<0 else 0; p=libc.syscall(434,os.getpid(),0); pe=ctypes.get_errno() if p<0 else 0; (os.close(p) if p>=0 else None); print(json.dumps({"landlock_abi":int(a),"landlock_errno":ae,"pidfd_open":int(p>=0),"pidfd_errno":pe}))'
```

| Result | What it means |
|---|---|
| `ldd` below 2.31 | The release build may not start. |
| `uname` not ending in `x86_64` | No qualified build exists. |
| `df` shows `nfs`, `nfs4`, `cifs` or `smb3` | See [Storage requirements](#storage-requirements). |
| `$HOME` mode `775`, `2770` or similar | Refused. Run `chmod go-w "$HOME"`. |
| `/etc/machine-id` missing or not owned by `root` | See [Machine identity](#machine-identity). |
| `pidfd_open` is `0` | `stop` fails with `unsupported: safe stop requires Linux pidfd_open`. Do not host here. |
| `landlock_abi` below `3` | Agents cannot run commands in a member's remote work folder. Those commands also need seccomp. Everything else works. |

### Machine identity

Each workspace is tied to one machine. The broker reads `/etc/machine-id`, or `/var/lib/dbus/machine-id`: a regular file owned by root, not writable by group or others, holding 32 hexadecimal characters that are not all zeros. Otherwise it stops with `node_identity_unavailable`, which only the server's administrator can fix.

### Server accounts

- The host runs the broker as an ordinary account. Root gets `unsupported: Crew must run as an ordinary user`.
- Each member uses their own account. Crew sees the accounts `getent passwd` shows, directory accounts included.
- System accounts cannot be invited: UID 0, UIDs below `UID_MIN` in `/etc/login.defs` (1000 when unset), UID 65534, and shells `nologin` or `false`.
- Crew shows the first GECOS field as the person's full name.

### SSH requirements

- Each member needs an SSH login to the machine running the broker, because other nodes cannot reach its socket. If your server name reaches several login nodes, give members that node's name and have them compare `ssh hpc.example.edu hostname` with the host's.
- A member who uses `biorouter crew` on that machine itself, such as a login node, still connects over SSH, to their own account there. They need their own key in their own `~/.ssh/authorized_keys` and the machine's host key in `~/.ssh/known_hosts` for `localhost`, and no jump host. [Join from the workspace's own server](command-line.md#join-from-the-workspaces-own-server) gives the steps. Such a node often has no keyring service, so they also need the encrypted vault ([Keep device keys in an encrypted vault](command-line.md#keep-device-keys-in-an-encrypted-vault)).
- Crew uses each member's own OpenSSH client and settings, and connects only to hosts already in `~/.ssh/known_hosts`. Give members the host key fingerprints of the server and every jump host, or add them to the members' known hosts files yourself.
- Jump hosts must use `ProxyJump`, with at most 16 hosts in a route. A custom `ProxyCommand` or `GSSAPIDelegateCredentials yes` is refused.
- Crew signs in without prompts (`BatchMode yes`) except in its Sign in window and `biorouter crew auth`, which turn `BatchMode` off for the server. That option does not reach a jump host, so a jump host that asks for a password or a code must not match a `BatchMode yes` setting in the member's SSH settings, or it can never ask.
- Crew adds the first five settings below to its server connection. Each jump host needs the whole stanza in the member's SSH settings, before broader defaults, beside your `HostName`, `User`, `Port` and `IdentityFile` lines. A missing one gives `Crew SSH host <host> (the jump host on this connection's route) requires <setting>; …`.

  ```text
  Host crew-gateway
      StrictHostKeyChecking yes
      ForwardAgent no
      ForwardX11 no
      PermitLocalCommand no
      ClearAllForwardings yes
      NoHostAuthenticationForLocalhost no
      GSSAPIDelegateCredentials no
      Tunnel no
      ForkAfterAuthentication no
      ControlMaster no
      ControlPath none
      ControlPersist no
  ```

- Because `StrictHostKeyChecking yes` is set for each jump host, a member's `ssh -J` never asks about a jump host whose key is not known yet. It stops with `No ED25519 host key is known for crew-gateway and you have requested strict checking.` Have each member verify a jump host directly first, with `ssh -o StrictHostKeyChecking=ask crew-gateway`, comparing the fingerprint you gave them, or add the key to their known hosts file. Then `ssh -J` asks about the server. [Verify the server on this computer](joining-a-workspace.md#verify-the-server-on-this-computer) walks a member through it.
- Joining a workspace gives nobody SSH access.

### Storage requirements

The state directory, by default `~/.local/share/biorouter-crew/<workspace name>` in the host's account, holds the workspace.

| Rule | Refusal |
|---|---|
| Local, persistent disk. Lustre, GPFS, BeeGFS and CephFS are not qualified, and `/tmp` is cleaned. With home folders on NFS, pass `--state-dir` with a local folder (untested). | `unsupported_storage` on NFS, CIFS or SMB |
| The directory belongs to the host, has mode 0700 and is no symbolic link. | `unsafe_storage: state directory must be owner-only and not a symlink` |
| No folder above it is writable by group or others unless sticky. | `unsafe_storage: writable non-sticky ancestor` |
| Each file belongs to the host, has mode 0600 and one link. | `unsafe_storage: file ownership, mode or links invalid` |
| One broker per workspace. | `writer_active` |

With `--state-dir`, the parent folder must exist. The socket is `/tmp/crew-<host UID>-<…>/broker.sock` (folder 0711, socket 0666). The broker checks every request and keeps the same path after `/tmp` is cleaned. If the folder's permissions changed but still kept other accounts from writing into it, the broker sets them back and keeps the path. If another account took that path while `/tmp` was empty, the broker moves to a new one and `start` prints `"socket_changed":true`. Members need to do nothing: Crew finds the new folder from the old path. If someone's Crew can't connect after a move, they update `~/.local/bin/biorouter-crew` in their server account. Don't send a new invitation line to people who already joined: Crew refuses a line that differs from the workspace it has saved.

## Plan before you host

1. Pick the host account. The role cannot move later.
2. Pick a workspace name of 1 to 40 lowercase letters, digits and hyphens. Anyone on the server can see it, so leave out patient and sample identifiers.
3. Pick the institution ID, such as `ucsf`. It cannot change once set, and agents cannot work in a Private workspace without it. One computer cannot use two workspaces on one server with different institutions. See [Privacy and security](privacy-and-security.md).

## Install biorouter-crew

Every account that connects, the host's included, needs its own copy at exactly `~/.local/bin/biorouter-crew`. A copy only elsewhere on `PATH` does not count. The person installs it signed in as themselves, or an administrator installs it into their account. The host's ordinary account cannot write into another account, so hosts send these requests to you. [Install Crew in your server account](joining-a-workspace.md#install-crew-in-your-server-account) is the member's view.

### Get a verified file

- Download `biorouter-cli_<version>_amd64.deb` or `biorouter-cli-<version>-1.x86_64.rpm` from the [Biorouter release page](https://github.com/BaranziniLab/biorouter/releases). Its `sha256sum` must match the `sha256:` value shown beside it.
- Either package installs `/usr/bin/biorouter-crew`. To extract it instead, run `dpkg-deb --extract biorouter-cli_<version>_amd64.deb staging` and use `staging/usr/bin/biorouter-crew`.
- Or build it on Linux with `cargo build --locked --release -p biorouter-crew`, following [Linux artifact portability](design/linux-portability.md).

[Install Crew on the server](hosting-a-workspace.md#install-crew-on-the-server) gives these steps for a person on a Mac.

### Install commands

Run these on the server as the account that needs the copy, with the file's path in the first line. If the server has `/usr/bin/biorouter-crew`, use `VERIFIED_BINARY="$(command -v biorouter-crew)"`.

```bash
VERIFIED_BINARY='/replace/with/path/to/verified/linux/biorouter-crew'
mkdir -p "$HOME/.local/bin"
install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" --version
```

The last line prints the version. The Host dialog shows these commands under **Crew isn’t on hpc.example.edu yet?**, and a member's "Crew isn’t set up" screen under **Install it yourself**. The Invite dialog shows the `command -v` form under **If Bob sees “Crew isn’t set up”**.

Without a copy, a member sees "Crew isn’t set up for your account on hpc.example.edu", and `biorouter crew connect` fails with `crew_bridge_missing`. After the install, they choose **Try again**.

## Start and stop the broker

In step 2 of the Host dialog, **Start it for me** runs the start commands over SSH, or the host runs them from **Run it yourself in a terminal**. Their `--bootstrap-key` is the host computer's public key, used only to create the workspace.

`start` runs the broker in the background, logs to `broker.log` in the state directory, and prints one line of JSON:

- `"state":"running"` with an `invitation` line that the host pastes into Crew. If `invitation` is `null`, `invitation_error` says why.
- `"state":"starting"`: no answer within 15 seconds. Wait, then run `status`.
- `start_failed`: the broker stopped. The last log line names the cause.

`status` checks that the broker answers as the same account and workspace, and prints one line of JSON with `"state":"running"` and the workspace's `name`. If the broker has stopped saving changes, `status` fails instead, with the cause and the commands to run (see [Server storage full or failing](#server-storage-full-or-failing)).

### After a server restart

Nothing restarts the broker when the server boots. The host starts it again with the steps in [After the server restarts](hosting-a-workspace.md#after-the-server-restarts).

### Stop the broker

```bash
"$HOME/.local/bin/biorouter-crew" stop --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
```

Only the host can stop it. `stop` confirms the process behind the socket is this broker, sends SIGTERM, waits up to 5 seconds and prints `"stopped":true`. After `"stopped":false`, check that the process is gone before you start again. Data is kept, and members see the workspace as offline.

### Server storage full or failing

When the disk that holds the state directory fills up, the host's disk quota runs out, or a write to it fails, the broker stops saving changes. It has to: a journal written past a fault could not be trusted. Reading still works, and the broker does not recover by itself.

- A member's post fails with "The workspace server can’t save messages right now. Ask {host} to restart Crew." In a terminal, the sentence says whether the change was saved, such as `The workspace server is out of disk space, so this change was not saved.` Its code is `storage_full` for a full disk or quota, and `storage_failed` for any other write fault.
- The host reads "The server is out of disk space. Free space on it, then restart Crew there." for a full disk, or "The workspace server can’t save messages right now. Restart Crew on the server, then send again." for another fault, and a terminal adds the commands to run.
- Once a member's Crew learns of it, when it next connects to the workspace or a change is refused, their connection bar says "The workspace server has stopped saving changes. Reading still works.", and `biorouter crew status` says the same. See [A server that stopped saving](connections-and-troubleshooting.md#a-server-that-stopped-saving).
- `broker.log` in the state directory gets one line with the time, the operating system's error and the commands.
- `status` fails with a line such as `storage_full: Crew on this server stopped saving changes at <time> because the disk is full. Reading still works.`, followed by the commands.

A shared file the broker cannot write is refused the same way, but the broker keeps saving other changes. The upload pauses with "The workspace server couldn’t save it", and resumes from where it stopped once there is space.

To recover, as the host:

1. Free space on that disk, or ask IT to raise your quota. For `storage_failed`, ask IT to check the disk.
2. Stop the broker as in [Stop the broker](#stop-the-broker), and check for `"stopped":true`.
3. Start it with `"$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"`. The `start` line prints `"state":"running"`, and the broker repairs the end of its journal as it starts.
4. Members' Crew reconnects by itself, and anyone who sees the workspace offline can choose **Connect to chen-lab**. Anyone whose change said it "may not have been saved" checks the channel before sending it again.

## How identity works

- The broker checks every request: the member is active, uses the UID they joined with, and that UID keeps its username. If not, the command line shows `Account enrollment changed.` and `biorouter crew members` shows `account no longer valid`. Usernames are unique among active members, ignoring case and lookalike letters.
- After you rename an account, remove the old member and invite the new name. Inviting the new name first is refused with `This account joined as @bob and is now @robert…`.
- Remove a member from every workspace before you delete their account. Crew cannot tell apart two people who get the same UID and username.
- Only the host manages membership, the workspace name, privacy and institution, and agents never can. See [Hosting a workspace](hosting-a-workspace.md).
- Removing someone ends every agent's access in the workspace. Their messages, server account and SSH access stay. Inviting them again creates a new member with no earlier teams or channels.

The host's account, programs running as it and the server's administrators can read everything the workspace stores, including Restricted channels. SSH encryption alone does not make a deployment HIPAA compliant. Your institution decides endpoint approval and access review.

## What other accounts can see

Crew keeps messages, files, member lists and channel names behind each member's device key. Any account that can sign in to the server, member or not, can still learn this much:

| What | How |
|---|---|
| The workspace's name | The host's `biorouter-crew serve` process shows `--name` or the state directory's path, and the state folder under the host's home has the same name. |
| Who is connected to which workspace, and when | While a member's computer is connected, `biorouter-crew bridge` runs under the member's username with `--socket`, `--owner-uid` and `--workspace-id` on its command line. `ps` lists it for every account. |
| The workspace's ID, name, privacy mode, institution, policy epoch and key fingerprint, and whether it has stopped saving changes | The broker's socket is mode 0666, because each member's bridge reaches it from their own account, and its `hello` answers any local account with these. |

To hide other accounts' processes, mount `/proc` with `hidepid=2` on a shared server, for example with `mount -o remount,hidepid=2 /proc` and the same option in `/etc/fstab`. Check first that your monitoring tools still work, since they may need a group given with `gid=`. Nothing hides the `hello` answer, so keep patient, sample and study identifiers out of workspace names.

On member computers, each Biorouter background service listens on a TCP port on the loopback address. On a shared Linux machine another account can reach that port, but every request except `/status` needs the service's secret or a short lived key it issued, and `/status` answers only `ok`.

## Member computers

- Each member needs the Biorouter app or command line, and the OpenSSH client.
- On macOS and Linux, the app and command line share one background service per profile, which keeps running after the app closes. The command line needs `biorouterd` beside `biorouter`. On Windows it cannot use the service: `Shared Crew daemon IPC is unavailable on this platform`.
- A [managed policy](../security/managed-policy.md) with a hook under `hooks` or `allow_project_hooks: true`, or one that does not parse, blocks agent tasks and chat access with "Crew is unavailable with required managed hooks…" or "…the managed policy could not be loaded…". Messages and files still work.
- Each person also keeps an approval secret. See [Getting started](getting-started.md).
- Crew keeps device keys in the system keyring, and never in a plain file. A Linux computer with no keyring service (Secret Service), such as a login node reached only over SSH, needs `biorouter crew credentials init` before its first workspace. See [Keep device keys in an encrypted vault](command-line.md#keep-device-keys-in-an-encrypted-vault).

## Where Crew keeps its data

| Path | Contents |
|---|---|
| `journal.jsonl` | The whole workspace and every change, appended with chained checksums. |
| `blobs/` | Shared file contents. |
| `writer.lock`, `runtime.json`, `broker.log`, `torn-tail-<ID>` | The lock, the broker's details, its log, and any journal line set aside during recovery. |
| `~/.local/bin/biorouter-crew` | The program, in each member's server account. |
| `~/.local/state/biorouter-crew/remote-jobs/` | Records of agent commands, in each member's server account. |
| `~/.config/biorouter/crew/` | On each computer: saved connections, and vault files when used. Device keys stay in the system keychain. |
| `~/.local/state/biorouter/crew/` | On each computer: `runs.json`, the record of the agent tasks started there, and `transfers/`, the receipts of its file uploads and downloads. |
| `~/.local/share/biorouter/crew/tasks/` | On each computer: the working folder of agent task chats. The task chats themselves are saved with your other chats, in `~/.local/share/biorouter/sessions/`. |
| `~/.local/state/biorouter/daemon/` | On each computer: how Biorouter finds its background service. |

The first three rows are in the state directory. Computer paths are macOS and Linux defaults.

## Upgrade Crew

Upgrade at a quiet time, because members disconnect while the broker restarts. As the host:

1. Install the new file beside the old one, and check it:

   ```bash
   cd "$HOME/.local/bin"
   cp -p biorouter-crew biorouter-crew.previous
   install -m 0755 /path/to/new/biorouter-crew .biorouter-crew.new
   sha256sum .biorouter-crew.new
   ./.biorouter-crew.new --version
   ```

2. Stop the broker and check for `"stopped":true`:

   ```bash
   ./biorouter-crew stop --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
   ```

3. Replace the file and start the broker:

   ```bash
   mv -f .biorouter-crew.new biorouter-crew
   ./biorouter-crew start --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
   ```

   The `start` line prints `"state":"running"`.

4. Tell members to choose **Connect to chen-lab** if the workspace shows as offline. Their status row reads "Connected" again.

The new broker reads the journal as it is. To roll back, work in `~/.local/bin`:

1. Stop the broker as in step 2 of the upgrade, and check for `"stopped":true`.
2. Run `mv -f biorouter-crew.previous biorouter-crew`.
3. Run `./biorouter-crew start --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"`. It prints `"state":"running"`.

Members replace their own copy the same way. It takes effect at their next connection. If a member reads that the server "can't let people join with a code yet" or "can't add people directly yet", upgrade the broker.

After a Biorouter update, each computer's background service stays old until it restarts, because reopening Biorouter on macOS or Linux attaches to the running one. "…needs a newer Biorouter background service…" in the app, or `Restart the shared Biorouter daemon to …` in a terminal, means this. [Replace an old background service](connections-and-troubleshooting.md#replace-an-old-background-service) gives the fix. Stopping the service affects every window and terminal using it.

## Back up a workspace

Crew has no export, backup, restore, retention or deletion tools, and never deletes messages or files. Your institution decides backup, retention, deletion and legal hold. To preserve a copy:

1. Stop the broker and check for `"stopped":true`.
2. Make sure nobody starts it while you copy.
3. Copy the whole state directory to equally private storage, and check the copy with `sha256sum`.
4. Start the broker again. The `start` line prints `"state":"running"`, and members can connect.

Never edit `journal.jsonl`, because a broker refuses records that fail their checksums. A copy cannot run on another machine (`node_identity_changed`) or account (`host_identity_changed`), and Crew has no failover. The host's account and root can change the journal, so keep tamper resistant audit records elsewhere.

## Workspace limits

| Item | Limit |
|---|---|
| Workspace state (live data) | 16 MiB |
| Journal file | 1 GiB |
| Messages | 100,000 |
| Records kept for retries | One day each, the newest 512 per person and 100,000 in all. Older ones are dropped, never refused. |
| Teams, channels | 100, 1,000 |
| Shared files | 1 GiB each, 10 GiB total, 10,000 files. |
| Unfinished uploads | Each holds its full declared size of the shared file space, and of its member's share, from the moment it starts. One with no progress for a day is removed, with its partial file. Cancelling it on the member's computer does not remove it sooner. |
| Remote references | 10,000 |
| One member's share (the host has none) | 4 MiB of workspace state, 256 MiB of journal, 10 teams, 100 channels, 2.5 GiB and 2,500 shared files, 2,500 remote references, 100 waiting invitations |
| Channels an agent reads | A task: its channel plus 16 more. A chat: 20, including its own. |
| Broker connections | 256, and 8 per account |
| One agent command on the server | 60 seconds of CPU, 1 GiB memory, 16 MiB files, 64 open files, 30 seconds of run time (60 at most), 4 at once |
| What one agent command can reach | Its work folder, and the programs and libraries under `/usr/bin`, `/usr/lib`, `/bin` and `/lib`. It has no network, cannot start other processes and cannot read `/etc`, so cluster tools such as `sbatch` and `squeue` do not run. See [Work in the remote work folder](agents-and-chat-access.md#work-in-the-remote-work-folder). |
| Agent file access in the remote work folder | 256 KiB per read or write, 64 KiB of command output |

The state limit decides capacity. Ordinary changes may use 15 MiB of it, so fewer than 15,360 messages of 1 KiB fit, or 1,920 of 8 KiB. Archiving frees nothing. At a limit, reading works and changes are refused with "This workspace has grown past the size Crew supports…". The host can still remove members and change the privacy mode, and anyone can still stop an agent, archive a channel or remove someone from a channel they own. Preserve it and start a new workspace with a new state directory.

A member who reaches their share sees "You have used your share of this workspace's storage" (or its journal, files, teams, channels, references or invitations). Everyone else keeps working, and reading still works for them too.

The host is warned before the workspace fills. From 80% of the space ordinary changes may use, in the state or the journal, whichever is fuller, the host's Crew sidebar shows a **Storage** row such as "chen-lab is 82% full." with what happens when it is full. The **General** tab of "chen-lab settings" always shows the host how full it is, with the same sentence from 80%. At the limit the row reads "chen-lab is full." Members are not told. A server with an older `biorouter-crew` sends no figures, and nothing is shown.

Plan for up to about 1.2 GiB of broker memory when 50 accounts post at once.

## Server messages

`biorouter-crew` prints `Error:` and the message.

| Message | What to do |
|---|---|
| `bootstrap_key must be a 32-byte Ed25519 public key` | The folder holds no workspace. Check the `--state-dir` path. |
| `name_mismatch: …` | Start with `--state-dir` and no `--name`. |
| `name_taken: …` | Another workspace you run uses the name. Choose another. |
| `not_running: …` | No broker is running from this state directory. Run `start`. |
| `journal_corrupt: checksum mismatch` | Change nothing. After an upgrade, roll back and use a release build. |
| `quota_exceeded: journal exceeds supported replay size of 1 GiB` | Preserve the workspace and start a new one. |
| `storage_full: Crew on this server stopped saving changes…` or `storage_failed: …` from `status` | See [Server storage full or failing](#server-storage-full-or-failing). |
| `ssh -G refused the native configuration; …` | Run `ssh -G <host>` on the member's computer to see the error. |

## Related documentation

- [Crew user manual](README.md): every Crew page.
- [Hosting a workspace](hosting-a-workspace.md): the host's steps in the app.
- [Command line](command-line.md): every `biorouter crew` command.
- [Privacy and security](privacy-and-security.md): privacy, institutions and keys.
- [Connections and troubleshooting](connections-and-troubleshooting.md): signing in and member messages.
- [Agents and chat access](agents-and-chat-access.md): agent tasks and the remote work folder.
- [Managed enterprise policy](../security/managed-policy.md): the policy file IT installs.
- [Linux artifact portability](design/linux-portability.md): how the Linux build is qualified.
- [Protocol and setup contract](design/protocol-contract.md): the broker's protocol and limits.
