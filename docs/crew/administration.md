# Crew administration

> **What this is.** The server side of Crew: what the Linux server needs, how to install the Crew program in each account, how Crew decides who a person is, how to upgrade, where the data lives, and the limits and setups Crew does not support.
> **Status:** Current. Describes Biorouter 1.91.2.
> **Audience:** Lab managers who host a workspace, and IT staff who look after the server it runs on. Most sections assume you are comfortable in a Linux terminal.

Crew is the part of Biorouter where a lab talks in channels, shares files and works with AI agents. A lab's Crew space is called a workspace. It lives on a Linux server that lab members already reach over SSH. One person, the host, runs a small program there under their own account. Every other member connects through their own SSH login to that server. Crew needs no administrator rights on the server, no system service and no open network port. This page collects what you need to set that up and keep it running. For the steps inside the app, see [Hosting a workspace](hosting-a-workspace.md).

The examples on this page use a host `@alice`, a member `@bob`, a server `hpc.example.edu`, a workspace named `chen-lab` and the institution ID `ucsf`.

## How Crew runs on the server

### The pieces

| Piece | What it is | Where it runs |
|---|---|---|
| Workspace | One lab's people, teams, channels, messages and shared files. | Stored in the host's account on the server. |
| Host | The person whose server account runs the workspace. The host invites people and sets the workspace's privacy. | An ordinary account on the server. |
| Broker | The program `biorouter-crew`, running as the host. It stores the workspace and checks every request. | The server, as the host's account. |
| Bridge | The same program, started over SSH in each member's account. It passes that member's requests to the broker. | The server, as each member's own account. |
| Background service | The part of Biorouter on each member's computer (`biorouterd`). It holds the device keys and opens the SSH connection. | Each member's computer. |
| Device key | A key pair Biorouter creates on each computer. It signs every request that computer sends. | The private half never leaves the computer. |

```text
 Member's computer                        Linux server (one machine)
+----------------------------+          +------------------------------------------+
| Biorouter app or CLI       |   SSH    | bridge, as bob:                          |
| background service         |  as bob  |   ~bob/.local/bin/biorouter-crew         |
| (bob's device key)         |--------->|        |                                 |
+----------------------------+          |        | Unix socket under /tmp          |
                                        |        v                                 |
                                        | broker, as alice (the host):             |
                                        |   ~alice/.local/bin/biorouter-crew       |
                                        | data:                                    |
                                        |   ~alice/.local/share/biorouter-crew/    |
                                        |   chen-lab/                              |
                                        +------------------------------------------+
```

- The broker listens only on a Unix socket in `/tmp`. Crew opens no network port on the server. Members reach the socket only through their own SSH login to the same machine.
- The broker learns which account is calling from the Linux kernel, never from anything the caller sends. See [How identity works](#how-identity-works).
- All workspace data sits in files that the host's account owns. There is no database server.

## Server requirements

### Operating system and processor

| Requirement | Detail |
|---|---|
| Operating system | Linux. On any other system the program refuses to run the broker or the bridge. Its help ends with `Broker and bridge operations require Linux.` |
| Processor | x86_64. It is the only architecture with a qualified release build. No ARM (aarch64) build is qualified. |
| C library | glibc 2.31 or newer. The release build targets glibc 2.31, the version in Debian 11. The build of this version was checked to start on Debian 11 and Ubuntu 24.04. Check a server with `ldd --version`. |
| Machine identity | A valid `/etc/machine-id`. See [Machine identity](#machine-identity). |

### Kernel features

| Feature | What needs it | Without it |
|---|---|---|
| Unix socket peer credentials (`SO_PEERCRED`) | Every connection to the broker. Standard on Linux. | The broker cannot run. |
| `pidfd_open` | `biorouter-crew stop`, the safe way to stop the broker | `stop` refuses with `unsupported: safe stop requires Linux pidfd_open`. Do not host on such a server. |
| Landlock ABI 3 or newer, and seccomp | Agents that run commands in a member's remote work folder | Crew refuses to run those commands. It never runs them with weaker limits. Chat, file sharing, and agents that read, write files and post still work. |

### Check a server before you host

Run these commands on the server, as the account that will host. They change nothing.

```bash
ldd --version | head -n 1
uname -srm
df -P -T "$HOME" | tail -n 1
stat -c '%a %U %n' "$HOME" /etc/machine-id
python3 -c 'import ctypes,json,os; libc=ctypes.CDLL(None,use_errno=True); libc.syscall.restype=ctypes.c_long; a=libc.syscall(444,None,0,1); ae=ctypes.get_errno() if a<0 else 0; p=libc.syscall(434,os.getpid(),0); pe=ctypes.get_errno() if p<0 else 0; (os.close(p) if p>=0 else None); print(json.dumps({"landlock_abi":int(a),"landlock_errno":ae,"pidfd_open":int(p>=0),"pidfd_errno":pe}))'
```

What to look for:

| Line | Good result | Problem |
|---|---|---|
| `ldd --version` | 2.31 or higher | Lower: the release build may not start. |
| `uname -srm` | Ends in `x86_64` | Anything else: no qualified build exists. |
| `df -P -T "$HOME"` | A local file system type, such as `ext4` or `xfs` | `nfs`, `nfs4`, `cifs` or `smb3`: the home folder cannot hold the workspace. See [Storage](#storage). |
| `stat` on `$HOME` | No group or other write permission, such as `700` or `755` | `775`, `2770` and similar: the broker refuses. See [Storage](#storage). |
| `stat` on `/etc/machine-id` | Owned by `root`, mode such as `444` or `644` | Missing, or writable by group or others: the broker refuses. |
| `landlock_abi` | `3` or higher | `1` or `2`, or `-1` with `landlock_errno` `38`: agents cannot run commands on this server. |
| `pidfd_open` | `1` | `0` with `pidfd_errno` `38`: `stop` does not work. Do not host here. |

The last command uses x86_64 system call numbers (444 and 434). It asks the kernel which Landlock version it supports and opens a process handle for itself. It does not apply a sandbox or touch any other process.

### Machine identity

The broker ties each workspace to one machine. It reads `/etc/machine-id`, or `/var/lib/dbus/machine-id` when the first file is missing. The file must meet all of these:

- It is a regular file owned by root.
- Its group and other accounts cannot write to it.
- It holds 32 hexadecimal characters that are not all zeros.

Crew publishes only a digest derived from this value, never the value itself. When the file is missing or does not qualify, the broker stops with a message that starts with `node_identity_unavailable:`.

### Accounts

- The host runs the broker as an ordinary account. Root is refused: `unsupported: Crew must run as an ordinary user`.
- Every member uses their own account on the server. Crew has no shared login.
- The broker looks accounts up with the standard calls `getpwnam_r` and `getpwuid_r`. It sees the same accounts that `getent passwd` shows, including directory accounts that NSS provides. It asks about one name or UID at a time and never lists the server's accounts. When the host invites someone, the broker keeps the result of that name lookup for 30 seconds.
- Crew refuses to invite system accounts. A system account is one with UID 0, a UID below `UID_MIN` in `/etc/login.defs` (1000 when that file does not set it), UID 65534 (`nobody`), or a login shell that is `nologin` or `false`. The host sees `@root is a system account on this server and can't join a workspace.`
- Crew shows the first field of an account's GECOS comment as the person's full name, such as "Bob Lee". It is a label and grants nothing.

### SSH

- Every member needs a working SSH login to the machine that runs the broker. The broker's socket lives in that machine's `/tmp`. A Unix socket on one machine cannot be reached from another, even when home folders are shared. If your server name points at several login nodes, give members the name of the one node that runs the broker. To check, ask each member to run `ssh hpc.example.edu hostname` and compare the answer with the host's.
- Crew uses the OpenSSH client (`ssh`) on each member's computer, with that member's own SSH settings: keys, passwords, verification codes and jump hosts. Members type passwords and codes in Biorouter's Sign in window, or at the prompts of `biorouter crew auth`. Crew never saves them.
- Crew connects only to a server whose host key is already in the member's known hosts file (`~/.ssh/known_hosts`). It never accepts a new or changed key by itself. Give members the host key fingerprints of the server and of any jump host, so they can check them. When a key is unknown, Biorouter shows "Can’t verify hpc.example.edu yet". When it changed, Biorouter shows "hpc.example.edu’s identity changed" and a **Copy details for IT** button.
- Crew adds these options to every connection to the server: `StrictHostKeyChecking=yes`, `ForwardAgent=no`, `ForwardX11=no`, `PermitLocalCommand=no` and `ClearAllForwardings=yes`.
- Jump hosts must use `ProxyJump`. A custom `ProxyCommand` is refused, and so is `GSSAPIDelegateCredentials yes`. A route may have at most 16 hosts.
- Options that Crew adds for the final server do not reach jump hosts. Each jump host needs its own settings. Put a stanza like this in the member's SSH config, before broader defaults, and keep your existing `HostName`, `User`, `Port` and `IdentityFile` lines:

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

  When a setting is missing, Crew names it, for example `Crew SSH host crew-gateway requires StrictHostKeyChecking yes; put this in its matching Host stanza before broader defaults`.
- The desktop's **Start it for me** signs in without prompts. It works only when the server accepts the host's SSH key without asking for a password or a code. Otherwise the host runs the start commands in a terminal.
- Joining a workspace gives nobody SSH access. You keep managing SSH access the way you do today.

### Storage

These rules apply to the host's state directory, the folder that holds the workspace. By default it is `~/.local/share/biorouter-crew/<workspace name>`.

- The state directory must be on local, persistent disk. The broker refuses NFS, CIFS and SMB: `unsupported_storage: network filesystem requires qualified single-writer fencing; use local persistent HOME storage`. Crew does not recognize other shared file systems such as Lustre, GPFS, BeeGFS or CephFS. They are not qualified, so do not use them. Do not use `/tmp` either, because it is cleaned.
- If home folders are on NFS, the default location cannot hold the workspace. The checks allow a `--state-dir` on local persistent disk that follows the rules below. This setup was not part of the tested configurations.
- The state directory must belong to the host's account, give no access to anyone else (mode 0700), and not be a symbolic link. Otherwise the broker refuses with `unsafe_storage: state directory must be owner-only and not a symlink`.
- No folder above the state directory may be writable by its group or by other accounts, unless it has the sticky bit (as `/tmp` does). A home folder with mode 0775 or 2770 fails with `unsafe_storage: writable non-sticky ancestor`. Remove group and other write permission from that folder (`chmod go-w`), or choose a state directory under folders that have none.
- Every file in the state directory must have mode 0600, belong to the host's account, and have exactly one link. Otherwise the broker refuses with `unsafe_storage: file ownership, mode or links invalid`.
- With `--name` and no `--state-dir`, the broker creates `~/.local/share/biorouter-crew/` with mode 0700 by itself. With `--state-dir`, the parent folder must already exist.
- The socket goes in a folder directly under `/tmp`, named `/tmp/crew-<host UID>-<32 hexadecimal characters>/broker.sock`. The folder has mode 0711 and the socket 0666, so that members' bridges can reach it. Every request through it is still checked by account and signature. The broker picks the folder name once, records it, and reuses it after restarts and after `/tmp` is cleaned.
- Only one broker can serve a workspace. A lock file, `writer.lock`, keeps a second broker out: `writer_active: another broker holds this workspace`.

### What Crew does not need

- No root and no sudo.
- No system service and no system packages.
- No new accounts or groups, and no change to home folder permissions beyond the storage rules above.
- No open network port and no firewall change.
- No database server, container runtime or Node on the server.

## Plan before you host

1. Choose the host. Pick the person who will manage membership. The host role stays with that server account for the life of the workspace. Crew cannot move it to another account later.
2. Choose the machine. It must meet the requirements above, and it must be the one node that every member reaches by SSH.
3. Choose the workspace name. Use 1 to 40 lowercase letters, digits and hyphens, starting and ending with a letter or digit. Anyone who can sign in to the server can see the workspace name, so keep patient and sample identifiers out of it.
4. Decide the institution ID, such as `ucsf`. It has 1 to 64 lowercase letters, digits, `_` or `-`, and starts with a letter or digit. Once the host sets it on the workspace, it cannot change: `Workspace institution cannot be cleared or changed; use a new workspace.` Agents cannot work in a Private workspace until it is set. See [Institutions](privacy-and-security.md#institutions).
5. Install `biorouter-crew` in the host's account and in every member's account. See [Install biorouter-crew](#install-biorouter-crew).
6. Collect the SSH host key fingerprints of the server and any jump host, so members can check them.
7. Decide how you will preserve the data and when you will start a new workspace. See [Back up and preserve a workspace](#back-up-and-preserve-a-workspace) and [Limits](#limits).

## Install biorouter-crew

### Where the file must be

Every account that uses the workspace, the host's included, needs its own copy at exactly `~/.local/bin/biorouter-crew`. Biorouter starts the bridge with that exact path over SSH:

```text
~/.local/bin/biorouter-crew bridge --stdio --socket <socket path> --owner-uid <host UID> --workspace-id <workspace ID>
```

A copy only in `/usr/bin`, in `/usr/local/bin` or elsewhere on `PATH` does not count. Installing needs no administrator rights. Each person can install their own copy, or IT can install it in each account.

### Get a verified file

Use one of these sources:

- The Linux command line packages on the Biorouter release page, `biorouter-cli_<version>_amd64.deb` and `biorouter-cli-<version>-1.x86_64.rpm`. They install `/usr/bin/biorouter-crew`. Copy that file into each account's `~/.local/bin`. To take the file out of the `.deb` without installing the package, run `dpkg-deb --extract biorouter-cli_<version>_amd64.deb staging` and copy `staging/usr/bin/biorouter-crew`.
- A build from source on Linux, made with `cargo build --locked --release -p biorouter-crew`. A binary built on macOS or Windows does not run on Linux. For a build that matches the release's glibc 2.31 floor, follow [Linux artifact portability](../research/biorouter-crew/linux-portability.md).

Before you install a file, compare its `sha256sum` with the value from the person or page that provided it.

### Install commands

Run these on the server, signed in as the account that needs the copy. Replace the path in the first line with the path of the verified file:

```bash
VERIFIED_BINARY='/replace/with/path/to/verified/linux/biorouter-crew'
mkdir -p "$HOME/.local/bin"
install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" --version
```

The last line prints the version, for example `biorouter-crew 1.91.2`. The Host dialog shows the same commands in its **Crew isn’t on hpc.example.edu yet?** section.

When the server already has `/usr/bin/biorouter-crew` from the package, this shorter form works as written. The host's Invite dialog offers it:

```bash
mkdir -p "$HOME/.local/bin"
install -m 0755 "$(command -v biorouter-crew)" "$HOME/.local/bin/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" --version
```

### When a member's copy is missing

A member whose copy is missing, or cannot run, sees these messages:

| Where | Message |
|---|---|
| Desktop, main area | "Crew isn’t set up for your account on hpc.example.edu" and "It’s installed once per account, usually by your host or IT team." |
| Desktop, offline screen | "Crew isn’t running for you on hpc.example.edu" |
| After signing in, desktop or command line | `Signed in, but Crew couldn't start on the server. Crew may not be set up for your account there.` |
| Command line, `connect` | A failure with code `crew_bridge_missing` |
| Host dialog, step 2 | "Crew isn’t installed on hpc.example.edu yet. Open “Crew isn’t on hpc.example.edu yet?” below." |

The desktop gives the member a message to copy and send. After the install, the member chooses **Try again**.

```text
Hi Alice, Crew isn’t set up for my account (@bob) on hpc.example.edu yet. Could you or IT install biorouter-crew in ~/.local/bin for me?
```

The host's Invite dialog has a folded section, "If Bob sees “Crew isn’t set up”". It holds the shorter install commands and the line "Send this to whoever runs hpc.example.edu, to run in @bob’s account:". For the member's side of this, see [Crew is not set up on the server](connections-and-troubleshooting.md#crew-is-not-set-up-on-the-server).

## Start and stop a workspace

### From the desktop

The host chooses **Host a new workspace** on the first Crew screen, or **Host a new workspace…** under **Add a workspace** in the workspace menu. Step 2 of the dialog, "Start Crew on hpc.example.edu", shows the commands that start the broker:

```bash
umask 077
mkdir -p "$HOME/.local/share/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" start --state-dir "$HOME/.local/share/biorouter-crew/chen-lab" --name chen-lab --bootstrap-key <64 hexadecimal characters>
"$HOME/.local/bin/biorouter-crew" status --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
```

The key is the public half of the host computer's hosting key. **Start it for me** runs exactly these commands over SSH, as the host, with the host's SSH settings. If the server asks for a password or a code, the host opens **Run it yourself in a terminal**, runs the commands there, and pastes what they print. The full walkthrough is in [Start Crew on the server](hosting-a-workspace.md#start-crew-on-the-server). The command line route is in [Host a workspace](command-line.md#host-a-workspace).

### What start prints

On success, `start` prints one line of JSON:

```text
{"started_pid":48213,"state":"running","status_command":"status","workspace_id":"<workspace ID>","name":"chen-lab","invitation":"brcrew1:<invitation data>"}
```

- `invitation` is the invitation line that the host pastes into Crew. When the broker cannot build it, `invitation` is `null` and `invitation_error` says why.
- The broker runs in the background, in its own session, and writes its output to `broker.log` in the state directory.
- If the broker has not answered within 15 seconds, `start` prints `{"started_pid":48213,"state":"starting","status_command":"status"}`. Wait a few seconds, then run `status`.
- If the broker stops during startup, `start` prints `start_failed: the broker stopped during startup (<exit status>); last log line: <line>`. The last log line names the cause.
- `start` refuses a name that another workspace of the same account already uses on this machine: `name_taken: Another workspace you host on this server is already using this name. Choose a different name.`
- The `--bootstrap-key` is used only when the workspace is created. For an existing workspace it is ignored.

`status` connects to the running broker and checks, within 3 seconds, that it answers as the same account and the same workspace. It then prints the details the broker recorded: `pid`, `socket`, `workspace_id`, `host_uid`, `protocol`, `node_id`, `workspace_public_key` and `workspace_key_fingerprint`.

### Stop the broker

```bash
"$HOME/.local/bin/biorouter-crew" stop --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
```

- Only the host's account can stop it: `forbidden: only host account can stop broker`.
- `stop` checks that the process behind the socket is the right broker, sends it SIGTERM, and waits up to 5 seconds. It prints `{"signal_sent":true,"stopped":true,"pid":48213,"workspace_id":"<workspace ID>"}`.
- `"stopped":false` means the signal arrived but the process had not exited yet. Wait a few seconds and check that the process is gone before you start the broker again.
- Stopping keeps all data. The next `start` with the same state directory continues where the workspace left off.
- While the broker is stopped, members see the workspace as offline.

`status` and `stop` also accept `--name chen-lab` in place of `--state-dir` when the workspace uses the default location.

### After a server restart

Crew installs no service, so nothing starts the broker after the server restarts. The host runs the same `start` command again, with the same state directory. The workspace ID, keys, socket path and history stay the same. Members whose workspace still shows as offline choose **Connect to chen-lab** on the offline screen, or run `biorouter crew connect`.

### Command reference

The help text, as `biorouter-crew --help` prints it:

```text
Usage: biorouter-crew start --name NAME --bootstrap-key HEX [--state-dir PATH]
       biorouter-crew serve|start|status|stop --state-dir PATH [--name NAME] [--bootstrap-key HEX]
       biorouter-crew bridge --stdio --socket PATH --owner-uid UID --workspace-id UUID
       biorouter-crew --version
--name names the workspace (lowercase letters, numbers and hyphens); without --state-dir its state lives in ~/.local/share/biorouter-crew/NAME. start prints the workspace's invitation line (brcrew1:...) to paste into Crew.
Broker and bridge operations require Linux.
```

| Command | What it does |
|---|---|
| `start` | Starts the broker in the background, waits up to 15 seconds for it to answer, and prints the result. |
| `serve` | Runs the broker in the foreground and prints its details. `start` uses it. |
| `status` | Checks the running broker and prints its details. |
| `stop` | Stops the broker safely. |
| `bridge` | Started by Biorouter over SSH in a member's account. Do not run it by hand. |
| `--version` | Prints the version, such as `biorouter-crew 1.91.2`. |

## How identity works

### The server account is the identity

When a member's bridge connects to the broker's socket, the Linux kernel tells the broker which account (UID) is calling. The broker never accepts a username or UID that a program sends as proof of anything. A workspace membership belongs to a server account.

### Each computer has a device key

Biorouter creates a device key (Ed25519) on each computer. The private half stays on that computer, in the system keychain by default, or in an optional encrypted vault. Each request is signed with it, over a challenge that works once and expires after 60 seconds. Members see their devices in **Keys and security…** in the You menu at the bottom of the Crew sidebar.

### Every request checks the account again

For every signed request, the broker checks three things:

1. The member is still active in the workspace.
2. The request comes from the same UID the member joined with.
3. The server's current name for that UID is still the username they joined with.

If any check fails, the request is refused. The command line shows `Account enrollment changed.`, and `biorouter crew members` marks the person `account no longer valid`.

### One active member for each username

Two active members can never share a username. Crew compares usernames without letter case, and treats letters that look alike as the same.

### The host

The host is the account that runs the broker. The computer that created the workspace holds the host's first device. Only the host can:

- invite people to the workspace and let them in,
- cancel invitations to join,
- remove people from the workspace,
- rename the workspace,
- change the workspace's privacy and set its institution,
- add any member to any team or channel.

An agent can never do these things, even when it has access to the workspace. Crew has no way to hand the host role to another account, and the host cannot be removed. The broker refuses to open the workspace's data as a different account (`host_identity_changed`).

### Joining with a code

1. The host invites a person by their server username. The invitation is valid for 24 hours. At most 100 people can wait to join at once.
2. The person pastes the invitation into Biorouter. Their computer works out a code of 16 letters and digits, such as `7QK2-M9XA-3JTP-WZ4D`, from its own device key and the workspace key. Nothing the server sends can change this code.
3. The person sends the code to the host. The host enters it in **Let in…**, or runs `biorouter crew enroll approve @bob CODE`.
4. The person's computer confirms the same code. Only then are they in. A saved code that does not match lets nobody in.

Joining gives a person a place in the workspace only. It does not add them to teams or channels, and it does not give SSH access. The steps are in [Hosting a workspace](hosting-a-workspace.md#invite-people) and [Joining a workspace](joining-a-workspace.md).

### More than one computer

A member who uses a second computer needs a second invitation as another device. In the Invite dialog, the host turns on **Add another device for @bob** and chooses **Invite**. On the command line: `biorouter crew enroll invite @bob --add-device`. The new computer joins with its own code.

### Removing someone

In the desktop, the host opens the workspace menu (the workspace name at the top of the Crew sidebar) and chooses **People…**. In the person's options menu the host chooses **Remove from chen-lab…**, types the person's username, and chooses **Remove from chen-lab**. On the command line: `biorouter crew enroll revoke @bob`.

What happens:

- The person's membership, devices and agent access stop working. Their messages stay in history.
- Every agent's access in the workspace ends, for every member. Members grant their chats access again.
- The removed person's computer shows "You’re no longer in chen-lab".
- Inviting them again later creates a new member with none of their earlier team or channel memberships.
- Their server account and SSH access do not change.

### Renamed, deleted and reused accounts

- If you rename an account on the server, that member's requests fail. The host removes the old member and invites them again under the new name. Until then, inviting the new name is refused with `This account joined as @bob and is now @robert on the server. Remove @bob first.`
- If you delete an account and later give the same UID and the same username to someone else, Crew cannot tell the two people apart. Remove the old member from every workspace before you delete or reuse an account.

### What the host and server administrators can read

The workspace is stored in plain files in the host's account. The host's account, any program running as that account, and the server's administrators can read everything the workspace stores. That includes Restricted channels, shared files and the record of every change. File permissions protect the workspace from other members' accounts. They do not protect it from the host's account or from root.

SSH encryption alone does not make a deployment HIPAA compliant. Your institution decides endpoint approval, retention, access review and backup. For what Crew enforces on each side, see [Where each rule is enforced](privacy-and-security.md#where-each-rule-is-enforced).

## Member computers

Each member needs:

- The Biorouter desktop app, or the `biorouter` command line.
- The OpenSSH client (`ssh`).
- On macOS and Linux, the app and the command line share one Biorouter background service for each Biorouter profile. It holds the device keys and SSH connections, and it keeps running after the app closes. The command line needs `biorouterd` in the same folder as `biorouter`. Otherwise it stops with `biorouterd must be installed alongside biorouter for shared Crew startup`.
- On Windows, the command line cannot attach to the shared background service: `Shared Crew daemon IPC is unavailable on this platform`.
- When IT installs a Biorouter managed policy, a policy that lists no hook under `hooks` and does not set `allow_project_hooks: true`, and that Biorouter can read and parse. Otherwise Crew refuses agent tasks and chat access on that computer. See the two managed policy rows in [Unsupported setups](#unsupported-setups).

Three separate secrets are involved. Keep them apart:

| Secret | What it is for | Rules |
|---|---|---|
| Approval secret | Proves that a person, not an agent, approved an action in the shared background service. | 32 to 4096 printable ASCII characters, with no spaces. Biorouter does not save it. Keep it in a password manager. |
| Vault passphrase (optional) | Unlocks an encrypted vault that holds the device keys in place of the system keychain. | 1 to 1024 bytes of text. It must differ from the approval secret. |
| SSH password and verification codes | Signing in to the server. | Typed only in Biorouter's Sign in window or at the prompts of `biorouter crew auth`. Never saved. |

Set up the encrypted vault in a new Crew profile, before any device key exists. Existing keychain keys are not moved into it. For the commands, see [The approval secret](command-line.md#the-approval-secret) and [Keep device keys in an encrypted vault](command-line.md#keep-device-keys-in-an-encrypted-vault).

## Where Crew keeps its data

### In the state directory

The default state directory is `~/.local/share/biorouter-crew/<workspace name>` in the host's account.

| File or folder | Contents |
|---|---|
| `journal.jsonl` | The whole workspace: people, teams, channels, messages, access grants and every change, in order. Crew only adds to it. Each record carries a checksum chained to the one before. |
| `blobs/` | The contents of shared files, one file each. |
| `writer.lock` | The lock that keeps a second broker out. |
| `runtime.json` | The running broker's process ID, socket path, workspace ID, host UID, machine digest and workspace key. |
| `broker.log` | The output of a broker started with `start`. |
| `torn-tail-<ID>` | An incomplete last journal line, set aside during recovery. Present only when one was found. |

### Elsewhere on the server

| Path | Account | Contents |
|---|---|---|
| `/tmp/crew-<host UID>-<32 hexadecimal characters>/broker.sock` | Host | The socket that bridges connect to. |
| `~/.local/bin/biorouter-crew` | Every member | The program. |
| `~/.local/state/biorouter-crew/remote-jobs/` | Every member | Records of commands that agents ran in that member's remote work folder. |

### On each member's computer

These are the default locations on macOS and Linux.

| Location | Contents |
|---|---|
| `~/.config/biorouter/crew/connections.json` | Saved connections: server, workspace ID, pinned workspace key and privacy choice. |
| `~/.config/biorouter/crew/credential-backend.json` and `credential-vault.json` | Present only when the encrypted vault is used. |
| The system keychain | Device private keys, by default. |
| `~/.local/state/biorouter/daemon/` | How the app and the command line find the shared background service. |

## Upgrade

### Upgrade the broker

Upgrade at a quiet time. Members are disconnected while the broker restarts. Run these steps as the host, on the server.

1. Put the new file beside the old one under a temporary name, keep a copy of the old one, and check the new one:

   ```bash
   cd "$HOME/.local/bin"
   cp -p biorouter-crew biorouter-crew.previous
   install -m 0755 /path/to/new/biorouter-crew .biorouter-crew.new
   sha256sum .biorouter-crew.new
   ./.biorouter-crew.new --version
   ```

2. Stop the broker. Check that the output says `"stopped":true`.

   ```bash
   ./biorouter-crew stop --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
   ```

3. Replace the file and start the broker with the same state directory:

   ```bash
   mv -f .biorouter-crew.new biorouter-crew
   ./biorouter-crew start --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
   ./biorouter-crew status --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
   ```

4. Tell members to connect again if their workspace shows as offline.

A newer broker reads the existing journal as it is and adds nothing to it when it starts. The workspace ID, keys, socket path, policy and history stay the same. In testing, journals stayed identical, byte for byte, across every upgrade and restart.

### Roll back

The upgrade test went from 1.91.1 to the new broker, back to 1.91.1, and forward again. Each broker read the records the other had written. To roll back, stop the broker, put the previous file back, and start it again:

```bash
cd "$HOME/.local/bin"
./biorouter-crew stop --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
mv -f biorouter-crew.previous biorouter-crew
./biorouter-crew start --state-dir "$HOME/.local/share/biorouter-crew/chen-lab"
```

### If the new broker refuses the journal

A broker built on its own from older source could not read existing journals. Its `start` failed with this line:

```text
Error: start_failed: the broker stopped during startup (exit status: 1); last log line: Error: journal_corrupt: checksum mismatch
```

It changed nothing on disk. Release builds, and builds of the current source, do not have this problem. If you see the message, roll back as described above, then get a release build.

### Members' copies

Each member's `~/.local/bin/biorouter-crew` is their bridge. Members do not have to upgrade at the same moment as the host. In the upgrade test, older clients kept working against the new broker. Replace a member's copy the same way: install it under a temporary name, check it, then `mv -f` it into place. SSH starts a new bridge for each connection, so the new copy takes effect at the member's next connection.

### Biorouter on members' computers

The shared background service keeps running after the app closes. After you update the Biorouter app or command line, it can still be the older version. These messages mean it is:

- `This feature needs a newer Biorouter background service. Quit and reopen Biorouter, or enter the workspace details below.`
- `Restart the shared Biorouter daemon to use names; IDs still work.`
- `Restart the shared Biorouter daemon to invite or join with an invitation.`

On the command line, stop the service with `biorouter crew daemon stop`. The next `biorouter crew` command starts the new version. Stopping it affects every app window and terminal attached to it.

### Signs of an older broker

These messages mean the host's broker is older than the members' Biorouter. Upgrading the broker removes them.

- `This workspace's server can't let people join with a code yet. Ask the host for an enrollment token instead.`
- `This workspace's server can't add people directly yet. Invite them instead: biorouter crew invites create @bob --team <team>`

## Back up and preserve a workspace

Crew has no export, backup, restore, retention or deletion tools. It never deletes messages or files. Archiving a channel or removing a person keeps their history. Your institution decides backup, retention, deletion and legal hold.

To preserve a copy of a workspace:

1. Stop the broker and check that the output says `"stopped":true`.
2. Make sure nobody starts the broker again while you copy.
3. Copy the whole state directory to storage that is equally private. Include `journal.jsonl`, `writer.lock`, `blobs/`, `runtime.json`, `broker.log` and any `torn-tail-` files.
4. Check the copy with ordinary checksums, such as `sha256sum`.
5. Start the broker again.

Never edit, trim or truncate `journal.jsonl`. Each record is chained to the one before by a checksum, and a broker refuses a journal whose records do not match their checksums.

Limits of a copy:

- A copy cannot run on another machine. The broker refuses with `node_identity_changed: workspace belongs to another writer node; automatic failover is disabled`.
- A copy cannot run under another account. The broker refuses with `host_identity_changed`.
- Crew has no automatic failover and no supported way to move a workspace to a new server.
- Restoring a copy onto the same machine and account was not tested.

The journal does not protect itself from its administrator. The host's account or root can change the files. If your audit records must resist changes, keep them in a separately controlled place.

## Limits

| Item | Limit |
|---|---|
| Workspace state (all live data, including message text) | 16 MiB |
| Journal file | 1 GiB. The broker also refuses to open a larger one. |
| Messages | 100,000 |
| One message | 65,536 bytes of text |
| Teams | 100 |
| Channels | 1,000 |
| Shared files | 1 GiB each, 10 GiB in total, 10,000 files |
| Remote references | 10,000 |
| Recorded operations kept for safe retries | 100,000 |
| People waiting to join | 100 |
| Invitation to join | 24 hours |
| Channels one agent task or chat grant may read | 20, including the channel it posts to |
| One agent access grant | 1 hour at most. The broker allows 1 to 3,600 seconds, and Biorouter asks for 3,600. |
| Connections to the broker | 256 in total, 8 per account. Further connections are dropped. |
| Idle connection | Closed after 5 minutes |
| One request or response | 1 MiB |
| Workspace name | 40 characters |
| Team name | 64 characters, 120 bytes |
| Channel name | 80 characters, 120 bytes |
| Display name | 64 characters, 120 bytes |
| Avatar | 12 characters |
| One agent command on the server | 60 seconds of CPU, files up to 16 MiB, 1 GiB of memory, 64 open files, 30 seconds of run time by default and 60 at most. At most 4 run at once. |
| Agent file read or write in the remote work folder | 256 KiB. Command output: 64 KiB. |

### Capacity

The 16 MiB state limit, not the 100,000 message count, sets how much a workspace holds. The text of each message counts about twice, once in the history and once in the record kept for safe retries. With messages of 1 KiB, fewer than 8,192 fit. With messages of 8 KiB, fewer than 1,024 fit. Archiving a channel frees no space.

When a limit is reached, reading keeps working and changes are refused. Members see `This workspace has grown past the size Crew supports and cannot take more changes. Ask the host about starting a new workspace.` To keep working, preserve the old workspace and start a new one, with a new state directory and new invitations. The old workspace stays readable.

### Measured load

A test broker served 50 real server accounts that posted 1,500 messages over 30 minutes, with no errors. 99% of messages completed within 904 ms. The broker's memory peaked at about 1.2 GiB (1,262,800 KiB), and the journal reached about 2.2 MB. This measured the broker alone with test accounts, an ARM build on Debian 13. No test ran 10, 30 or 50 people using the app at the same time.

## Unsupported setups

| Setup | What happens |
|---|---|
| Broker or bridge on macOS, Windows or another system | Refused. Linux only. |
| ARM (aarch64) Linux | No qualified release build. Only x86_64 is qualified. |
| glibc older than 2.31 | The release build may fail to start. |
| State directory on NFS, CIFS or SMB | Refused: `unsupported_storage: network filesystem requires qualified single-writer fencing; use local persistent HOME storage`. |
| State directory on Lustre, GPFS, BeeGFS or CephFS | Not detected and not qualified. Do not use. |
| Home folder writable by its group or others | Refused: `unsafe_storage: writable non-sticky ancestor`. |
| Running the broker as root | Refused: `unsupported: Crew must run as an ordinary user`. |
| Two brokers on one workspace | Refused: `writer_active: another broker holds this workspace`. |
| Moving a workspace to another machine, or automatic failover | Refused: `node_identity_changed: workspace belongs to another writer node; automatic failover is disabled`. |
| Opening a workspace as another account | Refused: `host_identity_changed`. |
| Moving the host role to another account | No such feature. |
| Changing a workspace's institution after it is set | Refused. Start a new workspace. |
| One computer using two workspaces on the same server with different institutions | Refused at connect: `You already use this server for <name> (<institution>). <workspace> uses <institution>; one computer can't mix institutions on the same server.` |
| Agent commands without Landlock ABI 3 and seccomp | Refused. Never run with weaker limits. |
| A Biorouter managed policy on a member's computer that lists any hook under `hooks` or sets `allow_project_hooks: true` | On that computer, starting an agent task, granting a chat access, and each turn in a connected chat are refused: `Crew is unavailable with required managed hooks. Contact your administrator for a compatible managed policy.` Messages and files still work. Remove the hooks and the `allow_project_hooks: true` line from the policy. See [Managed enterprise policy](../security/managed-policy.md). |
| A Biorouter managed policy on a member's computer that cannot be read or parsed | The same actions are refused: `Crew is unavailable because the managed policy could not be loaded. Contact your administrator.` Fix the file so it reads and parses. A policy file that fails the ownership check is ignored and does not cause this refusal. |
| Kernel without `pidfd_open` | `stop` is refused. Do not host there. |
| Members reaching different login nodes | The broker's socket exists on one node only. Every member must reach that node. |
| Jump host through a custom `ProxyCommand` | Refused. Use `ProxyJump`. |
| Command line attached to the shared background service on Windows | Not implemented: `Shared Crew daemon IPC is unavailable on this platform`. |
| Automatic restart after a server restart | None. The host runs `start` again. |
| Export, restore, retention, or deleting messages and files | No such tools. |

### What was tested

The latest acceptance testing covered:

- Linux x86_64 brokers on a disposable server, and the broker build started on Debian 11 and Ubuntu 24.04.
- macOS desktops only. Windows and Linux desktops, and the shared command line service on Windows, were not tested.
- SSH with keys only. Passwords with verification codes (MFA), jump hosts and changed host keys were not tested live.
- No test of disk faults, power loss or several brokers writing at once.

## Server messages and what to do

The `biorouter-crew` program prints `Error:` followed by the message. `start` repeats the broker's last log line inside `start_failed: ...`.

| Message | Cause | What to do |
|---|---|---|
| `--state-dir required (existing private parent under HOME), or --name to use ~/.local/share/biorouter-crew/NAME` | Neither `--state-dir` nor `--name` was given. | Add one of them. |
| `bootstrap_key must be a 32-byte Ed25519 public key` | The broker found no workspace in the state directory, so it tried to create one, and the host key was missing or invalid. A mistyped `--state-dir` path has this effect. | Check the `--state-dir` path. For a new workspace, copy the start command from the Host dialog. |
| `name_mismatch: this workspace already has another name; start it without --name, or rename it in Crew` | `--name` differs from the name the workspace already has. | Start it without `--name`, or with its current name. |
| `name_taken: Another workspace you host on this server is already using this name. Choose a different name.` | Another running workspace of the same account uses this name. | Choose another name. |
| `unsupported_storage: network filesystem requires qualified single-writer fencing; use local persistent HOME storage` | The state directory is on NFS, CIFS or SMB. | Use local persistent disk. |
| `unsafe_storage: state directory must be owner-only and not a symlink` | The state directory is a link, belongs to someone else, or other accounts can open it. | Use a real folder owned by the host with mode 0700. |
| `unsafe_storage: writable non-sticky ancestor` | A folder above the state directory is writable by its group or by others. | Remove that write permission (`chmod go-w`), or move the state directory. |
| `unsafe_storage: file ownership, mode or links invalid` | A file in the state directory has the wrong owner or mode, or extra hard links, for example after another account copied files in. | Give each file the host as owner, mode 0600 and a single link. |
| `writer_active: another broker holds this workspace` | A broker for this workspace is already running. | Run `status`. Stop it first if you meant to restart it. |
| `node_identity_unavailable: Linux machine-id is required; ask the host operator to provide its standard node identity` | No usable `/etc/machine-id`. | Ask the server's administrator to provide the standard machine ID file. |
| `node_identity_changed: workspace belongs to another writer node; automatic failover is disabled` | The state directory was created on another machine, or the machine ID changed. | Run the workspace on its original machine. |
| `host_identity_changed` | The state directory belongs to a workspace created by another account. | Run it as the account that created it. |
| `unsupported: Crew must run as an ordinary user` | The broker was started as root. | Start it as the host's own account. |
| `forbidden: only host account can stop broker` | `stop` was run by an account other than the host. | Run it as the host. |
| `unsupported: safe stop requires Linux pidfd_open` | The kernel lacks `pidfd_open`. | Host on a server that has it. |
| `journal_corrupt: checksum mismatch` | The broker could not verify the journal's checksums. | Leave the state directory as it is. If this follows an upgrade, roll back to the previous file. See [If the new broker refuses the journal](#if-the-new-broker-refuses-the-journal). |
| `quota_exceeded: journal exceeds supported replay size of 1 GiB` | The journal passed 1 GiB. | Preserve the workspace and start a new one. |
| `Crew SSH host <host> requires <setting>; put this in its matching Host stanza before broader defaults` | A member's SSH settings for a server or jump host lack a setting Crew requires. | Add the named setting to that host's stanza. See [SSH](#ssh). |
| `Crew SSH host <host> uses a custom ProxyCommand; use native ProxyJump so every SSH hop can be checked` | The route uses `ProxyCommand`. | Use `ProxyJump`. |
| `ssh -G refused the native configuration; inspect it with your SSH administrator` | OpenSSH could not read the member's SSH configuration. | Run `ssh -G <host>` on the member's computer to see the error, then fix the configuration. |

## Related documentation

- [Crew user manual](README.md): the index of every Crew page.
- [Hosting a workspace](hosting-a-workspace.md): the host's steps in the app, from the Host dialog to letting people in.
- [Joining a workspace](joining-a-workspace.md): what members do with an invitation.
- [Privacy and security](privacy-and-security.md): privacy settings, institutions, keys and what each side enforces.
- [Connections and troubleshooting](connections-and-troubleshooting.md): connection states, sign in and the messages members see.
- [Command line](command-line.md): every `biorouter crew` command, including the host commands.
- [Agents and chat access](agents-and-chat-access.md): agent tasks, chat grants and the remote work folder.
- [Managed enterprise policy](../security/managed-policy.md): the policy file IT installs, and the `hooks` and `allow_project_hooks` keys that block Crew agents.
- [Linux artifact portability](../research/biorouter-crew/linux-portability.md): how the Linux build is made and qualified.
- [Protocol and setup contract](../research/biorouter-crew/protocol-contract.md): the broker's protocol, identity rules and capacity limits in full.
- [SSH hop policy](../research/biorouter-crew/ssh-hop-policy.md): the checks Crew applies to jump hosts.
- [Institutional SSH compatibility probe](../research/biorouter-crew/institutional-ssh-compatibility.md): the kernel probe and results from two institutional servers.
