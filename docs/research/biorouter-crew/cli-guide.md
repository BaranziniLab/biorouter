# BioRouter Crew: native CLI and shared desktop daemon

> **What this is.** How to use `biorouter crew` from a terminal: the shared desktop daemon and its credentials, the per-account remote executable, joining a workspace, naming people, teams and channels, messages, files, agents, chat grants and recovery.
> **Status:** Current. Describes the source on branch `codex/biorouter-crew` as of 2026-09-24, including name selection and joining by invitation (package CLI-CMD, `6abace05` and `d7296080`). It is not a release or end-to-end acceptance claim: those commands have been tested against a scripted daemon only, and joining by invitation also needs a broker built with the `join-by-name` feature, on by default since 2026-09-25 (see [Join a workspace](#join-a-workspace)).
> **Audience:** People who use Crew from a terminal, and developers and testers of the Crew CLI.

The native CLI and the desktop Crew panel call the same daemon services for connections, membership, transfers, policy and owned tasks, and the same daemon resolves the names both of them accept. Crew is BioRouter's shared workspace for a lab: a small broker process runs under one member's Unix account on a Linux server, and every member reaches it over their own SSH login.

Shared profile IPC currently requires Unix: an authenticated, owner-protected Unix socket connects clients to `biorouterd`. Native Windows shared-daemon attachment is not implemented; there is no TCP fallback for the CLI. The remote Crew broker and bridge currently require Linux. A workspace host must start that broker before anyone connects.

## Start the daemon and choose credential storage

```bash
biorouter crew --help
biorouter crew daemon status
biorouter crew daemon start
biorouter crew credentials status
```

`daemon status` checks the discovered instance without asking for the human approval secret; it reports an error if no instance is available. Run `daemon start` only when the daemon is absent: it refuses an already running instance. Ordinary Crew commands require the approval secret and can start a missing daemon; add `--no-start` to require an existing instance. Credential commands require an existing daemon.

The CLI and desktop must use the same BioRouter profile to share an instance. On Unix, the desktop normally discovers or starts this profile daemon and asks for its approval secret. Closing the desktop or exiting the CLI leaves the shared daemon running. Explicitly stop it with `biorouter crew daemon stop`; that affects every client attached to the instance. An explicit desktop shared-daemon opt-out or an external backend is a different deployment mode.

In the desktop Crew view, select the same saved connection, sign in when SSH asks for a password or MFA, and pick a team and channel for messages and tasks. Keys and security manages the same keyring or vault backend the CLI uses. Desktop file pickers and CLI file paths both register selections with the daemon's transfer service; switching clients does not create a separate workspace or transfer engine.

There are three separate credentials:

| Credential | Purpose and input |
| --- | --- |
| Human approval secret | Authorizes human operations in this daemon. Choose and retain it separately when starting a new instance; supply the same secret when attaching. It must contain 32–4096 printable ASCII characters without spaces. It is not recovered from daemon discovery metadata or desktop settings. |
| Optional encrypted-vault passphrase | Unlocks this profile's Crew device credentials. It must differ from the approval secret; the vault accepts 1–1024 UTF-8 bytes. The OS keyring is the default credential backend. |
| SSH credentials and MFA responses | Enter only in the native SSH authentication terminal. They are not Crew messages, agent prompts, device codes, enrollment tokens or vault passphrases. |

To use an encrypted vault, initialize it **before preparing identities or saving connections in a fresh Crew profile**. Existing keyring identities are not migrated by this command:

```bash
biorouter crew credentials init
biorouter crew credentials lock
biorouter crew credentials unlock
```

The default secret prompts hide input. `--approval-key-stdin` explicitly reads exactly the first stdin line as the approval secret. For `credentials init` or `credentials unlock`, the second line supplies the distinct vault passphrase. With `send --input -`, `connections save -` or `connections join-invitation -`, the remaining stdin content is the message, the JSON descriptor or the pasted invitation. Supply such input through a trusted secret-input pipe; do not put secrets in command arguments, shell history, environment variables or connection JSON. Native `auth` still requires an interactive terminal, so use its normal hidden approval prompt.

For privacy-sensitive actions, add `--expected-mode private` or `--expected-mode public`, for example `biorouter crew --connection lab --expected-mode private send methods --text 'Hello'`. The daemon refuses a mismatched saved mode instead of changing it. This optional expectation applies to `send`, `tasks start`, `grants grant`, and file upload, download and resume selection; omission preserves existing behavior. It does not change the connection's privacy setting or apply to file cleanup.

## Install the remote executable before connecting

Every connecting ordinary Unix account on the Linux SSH host needs its own executable at **`~/.local/bin/biorouter-crew`**. The daemon invokes that exact path for `bridge --stdio`; an executable only in `/usr/local/bin` or `/usr/bin`, or available only through `PATH`, does not satisfy this prerequisite. The account hosting the broker needs it too.

Obtain and verify a `biorouter-crew` artifact qualified for the target Linux architecture and runtime. In each connecting account's own SSH session, install the verified file without administrator privileges:

```bash
# Run on the remote Linux host as each connecting ordinary user.
VERIFIED_BINARY='/replace/with/path/to/verified/linux/biorouter-crew'
mkdir -p "$HOME/.local/bin"
install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" --version
```

If `connect` fails with code `crew_bridge_missing` (the desktop says Crew isn't set up for your account on the server), this file is missing or not executable. An agent's remote work folder (`remote_root`) may never be, contain or sit inside `~/.local/bin`, `~/bin`, `~/.bashrc.d`, `~/.profile.d`, the folder holding the bridge executable, or the credential and Crew state folders: the bridge refuses remote file and command operations there (`invalid_scope`) and suggests a folder such as `~/crew-work/lab`.

Use qualified local persistent storage and an institution-approved SSH route. Check [Linux artifact portability](linux-portability.md) and [institutional SSH requirements](institutional-ssh-compatibility.md) before deployment; version output alone does not qualify kernel, filesystem, confinement or institution policy. Do not infer compatibility with an older cluster runtime from the current ARM64 test artifact's GLIBC 2.39 requirement.

## Name people, teams, channels and connections

Commands take names. The shared daemon looks each name up in your own view of the workspace (the same `POST /crew/resolve` the desktop uses), so it can only ever find things you could already see. IDs are still accepted everywhere a name is.

| You type | Selects | Matched against |
| --- | --- | --- |
| `@bob` | A person | Active members' usernames on the server, exactly |
| `@bob` with `--former` (`remove-member` only) | Someone who has left the workspace | Former members your view of the workspace still lists |
| `analysis-lab`, `'Analysis Lab'` | A team | Teams you belong to, by name or handle; case, spaces, dashes and dots don't matter |
| `methods`, `'#methods'` | A channel | Your channels in every team; ambiguous when two teams have one |
| `analysis-lab/methods` | A channel in one team | Your channels in that team |
| `--connection lab`, `--connection bob@hpc` | A saved connection | Saved connections, by name (any case) or SSH target |
| A UUID or 64 hex characters | Anything | Always treated as an ID, never looked up as a name; the broker still authorizes it |

- **A person is always `@username`**, the account name on the server. A display name never selects anyone, so a nickname that looks like someone else's cannot redirect a command. A username that differs only in letter case is refused with `Did you mean @bob?`, never corrected silently.
- **Nothing is guessed.** An unknown name prints one line saying so and lists nothing. An ambiguous name lists only candidates you can see and says how to narrow it (for a channel, name its team: `analysis-lab/general`). Either way the command sends nothing and exits with status 1. When several names are wrong, each gets its own line.
- **Person-targeted changes are checked twice.** `invites create`, `members add`, `remove-member`, `ownership offer` and `enroll revoke` send the `@username` you typed along with the ID it resolved to, and the broker refuses the change (`target_mismatch`) if that account is no longer the one you named.
- **Quote a `#`.** In bash, and in zsh with `interactivecomments`, an unquoted `#methods` starts a comment and the command loses its argument. Write `'#methods'` or leave the `#` out: `methods` works everywhere. `@bob` needs no quotes.
- **There is no current team.** A channel name that exists in two of your teams (every team has a `general`) is ambiguous until you qualify it.
- **An older daemon** answers names with `Restart the shared Biorouter daemon to use names; IDs still work.` Restart it, or pass IDs.

Text output names people as `"Bob Lee" (@bob)`, or `@bob` alone when the display name is just the username, and channels as `#methods`; it hides machine IDs.

### Scripting with IDs

Scripts can keep passing IDs: UUID-shaped arguments never reach the resolver, so they work against any daemon, including one that predates names. To see IDs, add `--show-ids`, which appends them to text output, or use `--output-format json`, which always carries them. Where they come from:

- A saved connection's ID is `id` in `connections list` and `connections show`.
- `teams create` returns `team.id` and `channel.id`; `channels create` returns `id`; `members --show-ids` lists principal IDs.
- `files upload` and `files download` return a transfer receipt with `id`, and a completed upload carries `blob_id`. Task starts return `run_id`.

Files, transfers, tasks and chat sessions are never looked up by name: `files download` and `send --attachment` take a `blob_id`, `files status|watch|pause|resume|forget` a transfer ID, `tasks show|watch|cancel` a run ID, and `context`, `grants grant` and `grants revoke` a chat session ID. `enroll revoke` given an ID never asks for confirmation, so a script can use it.

```bash
CONNECTION_ID='replace-with-saved-connection-id'
CHANNEL_ID='replace-with-channel-id'
biorouter crew --connection "$CONNECTION_ID" --output-format json history "$CHANNEL_ID" --latest --limit 50
```

## Join a workspace

The host starts the broker once and invites each person by their username on the server. A person joins by pasting the host's invitation, signing in, and sending the host a 16-character code their own computer computed; the host types that code to let them in. Nobody copies a key, a socket path or a numeric user ID.

> **Note.** Letting someone in with a code needs a broker built with the `join-by-name` feature; it advertises `join_by_name_v1`. The feature is on by default since 2026-09-25, after its adversarial review and live multi-account runs passed (naming decision D17), so a broker built without feature flags has it. Against a broker built with `--no-default-features`, or one from before the feature, `crew join` says the server can't let people join with a code yet, `enroll invite @bob` is refused, and the host uses the [older enrollment token](#enroll-with-a-token-older-versions) instead. Saving a connection from an invitation works with every broker.

### Host a workspace

Prepare this computer's hosting identity, then start the broker on the server as your own account with the public key it printed. `--name` names the workspace: 1–40 lowercase letters, digits and dashes. With `--name`, the state directory defaults to `~/.local/share/biorouter-crew/<name>`; `--state-dir PATH` still sets it explicitly.

```bash
biorouter crew connections prepare
# Note preparation_id and public_key from the output.

# On the server, as the hosting account:
"$HOME/.local/bin/biorouter-crew" start --name lab --bootstrap-key 'PUBLIC_KEY_HEX'
```

`start` waits for the broker to answer and prints JSON whose `invitation` field is a `brcrew1:` line. Save your own connection from it, naming how you reach the server (an alias from your SSH settings, or `alice@hpc.ucsf.edu`) and the institution for a Private workspace. Then sign in, claim the workspace with the key you prepared, and confirm its institution label:

```bash
biorouter crew connections join-invitation ./lab-start.txt \
  --preparation-id 'PREPARATION_ID' --ssh-target hpc --institution ucsf
biorouter crew --connection lab auth
biorouter crew --connection lab workspace bootstrap
biorouter crew --connection lab privacy set-workspace private --institution ucsf
```

The institution label is permanent. Confirm it after `workspace bootstrap` and before any agent task or grant. The [rootless setup checklist](protocol-contract.md#rootless-setup-checklist) lists the broker's lifecycle commands (`status`, `stop`) and what `start` checks.

### Invite someone and let them in (host)

```bash
biorouter crew --connection lab enroll invite @bob
```

The host's broker checks that `bob` is an account on the server, looking up that one name and never listing accounts. The server's own accounts can't be invited: `root`, any account below the server's `UID_MIN` (from `/etc/login.defs`, 1000 when it doesn't say), `nobody`, and any account whose login shell is `nologin` or `false` are refused with `@root is a system account on this server and can't join a workspace.` The command prints `Invited @bob · "Bob Lee" (name on the server account).` and then the invitation message to send Bob:

```text
Join lab on Crew.
In Biorouter, open Crew, choose Join a workspace, and paste this whole message.
brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ
```

`connections invitation --for @bob` prints the message again. An invitation carries the workspace key, the server's address, and the workspace's privacy mode and institution; it holds no secret, and forwarding it to the wrong person gains them nothing without an account you invited and your approval of their code.

When Bob sends you his code, check who is waiting and let him in. Type the code as he sent it; case, spaces and dashes don't matter:

```bash
biorouter crew --connection lab enroll pending
biorouter crew --connection lab enroll approve @bob 7QK2-M9XA-3JTP-WZ4D
```

- **Saving a code is not letting someone in yet.** `enroll approve` answers `Code saved. @bob joins when their computer confirms the same code.` The broker compares the code only when Bob's computer claims it, so a code you mistyped is caught then, not here.
- **Approve only a code the person sent you themselves.** If `enroll pending` says `A computer trying to join as @bob showed a different code. Check the code @bob sent you; if you typed it wrong, run enroll approve again with --replace. Don't approve a code you didn't get from @bob.`, either you typed Bob's code wrong or something other than Bob's computer tried to join. Compare with the code Bob sent you; never approve one you did not get from him.
- `--replace` replaces a code you already entered. Running `enroll approve` again with a different code and no `--replace` is refused with `You already entered a code for @bob.` and, on the next line, the command that replaces it. `enroll cancel @bob` withdraws the invitation. An invitation expires after 24 hours; inviting again replaces it, and Bob's screen then shows a new status.
- `enroll invite @bob --add-device` invites an existing member to add another computer.

### Join (joiner)

Save the host's message to a file, check what it says, save the connection, sign in and join:

```bash
biorouter crew connections join-invitation ./lab-invitation.txt --preview
biorouter crew connections join-invitation ./lab-invitation.txt
biorouter crew --connection lab auth
biorouter crew --connection lab join
```

- **The preview is the privacy decision.** It names the workspace, the host and the server, shows the fingerprint and the workspace's privacy (`Workspace privacy: Private · ucsf`), and says how this computer will treat it (`You'll join as Private · ucsf.`). If you choose another institution than the workspace's, it says so (`lab uses ucsf; you chose foreign-lab.`). One computer can't use one server for two institutions, so when another saved connection already reaches the same server under a different institution, the preview warns before you save: `You already use this server for foreign-lab (foreign-synthetic). lab uses ucsf; one computer can't mix institutions on the same server.` Connecting such a connection is refused in the same words. Without `--yes` the command asks `Save this connection? [y/N]` before saving anything. `--mode` and `--institution` change your own choice; `--username` sets your username on the server (default: the one the host invited); `--name` names the connection on this computer (default: the workspace's name). `--ssh-target`, `--port`, `--identity-file` and `--proxy-jump` (empty for none) override the server hints.
- **Pasting instead of a file:** `connections join-invitation -` reads the message from stdin (end it with Ctrl-D). Because there is then no terminal to ask in, it saves only with `--yes`; check it with `--preview` first.
- **`crew join` prints your code and waits:** `"Alice Chen" (@alice) invited you to lab.` and `Send Alice this code: 7QK2-M9XA-3JTP-WZ4D`. Your computer computes the code from its own device key and the workspace key in the invitation; nothing the server sends can change it. When Alice approves it, `join` finishes with `You're in lab.` Ctrl-C stops waiting and leaves the invitation open; run `join` again to continue. `--no-wait` prints the current state and returns, for scripts.
- If Alice typed a different code, `join` says `The code @alice entered doesn't match this computer. Send it again: …` with your code. It never prints `Joining lab…` for a code that doesn't match: once Alice saves a code, `join` claims first and then reports what the claim found, and with `--no-wait` it says `Alice saved a code for you, but this computer hasn't joined lab yet. Run biorouter crew join to finish.` when the claim hasn't completed. If you were not invited, it says so and waits for an invitation; an expired invitation ends the command with status 1 and asks you to request a new one. Sign in with `auth` before the first `join`.

An older shared daemon answers these commands with `Restart the shared Biorouter daemon to invite or join with an invitation.`

### Enroll with a token (older versions)

The enrollment token is the path for a host or joiner on an older version of Biorouter, and for a broker without `join_by_name_v1`. It remains until two releases after joining by invitation ships enabled. Its commands are hidden from `--help` and print a deprecation notice.

1. The joiner saves a connection (from the host's invitation, or from a [descriptor](#save-a-connection-from-a-descriptor)) and sends the host its device public key, `public_key` in `connections show`.
2. The host finds the joiner's numeric user ID (`id -u bob` on the server) and issues a token:
   `biorouter crew --connection lab enroll invite --uid 12345 --public-key 'DEVICE_PUBLIC_KEY_HEX'`. For another device of an existing member, add `--existing-principal PRINCIPAL_ID` (from `members --show-ids`).
3. The host delivers the token through an approved private channel, and the joiner accepts it at the hidden prompt: `biorouter crew --connection lab enroll accept`. `--token-stdin` reads it from stdin (the line after the approval secret when `--approval-key-stdin` is also given); `--token-fd FD` reads it from an explicit file descriptor.

The authenticated remote account and the enrolled key establish identity in both paths; display names do not change that authority.

### Save a connection from a descriptor

Use a JSON descriptor to script a save, or when the host gave you the workspace details rather than an invitation. `connections prepare` and `enroll prepare` are aliases for the same operation:

```bash
biorouter crew connections prepare
biorouter crew connections save ./crew-connection.json
biorouter crew connections list
```

Build `crew-connection.json` from verified workspace information supplied by its host. It is a JSON object with these fields; it contains no device private key or password:

| Field | Value |
| --- | --- |
| `preparation_id` | The ID from `connections prepare`, to use the device public key already shared for enrollment. Use only when saving a new connection. |
| `name` | A display name. |
| `ssh_target` | Your SSH host alias or `user@host`, using your own remote account. |
| `socket_path` | The broker's verified absolute remote socket path. |
| `owner_uid` | Numeric Unix UID of the remote broker owner; this is not necessarily your own SSH UID. |
| `workspace_id` | The host's verified workspace UUID. |
| `workspace_public_key` | The broker's verified 32-byte public key encoded as 64 hexadecimal characters. This is different from your prepared device public key. |
| `mode` | `private` or `public`; omission defaults to `private`. |
| `institution_id` | Required for private saves, optional for public. Canonical 1–64 lowercase ASCII letters, digits, underscores or hyphens, starting with a letter or digit (for example `ucsf`). Omitting `mode` still requires this field because the default is private. |

Optional fields are `port` (number), `identity_file` (absolute local path), `proxy_jump` (SSH jump route), `remote_root` (remote work directory), `remote_execution` (boolean, default `false`) and `cluster_connection_id` (existing cluster connection ID). Unknown fields are rejected. `connections show` includes read-only state, so its entire output is not a valid save or update descriptor. For updates, omit `preparation_id` and supply the complete editable descriptor.

### Sign in and manage saved connections

```bash
biorouter crew --connection lab connections show
biorouter crew --connection lab auth
```

`auth` opens the daemon-owned native SSH terminal for host authentication, passwords and MFA, including admitted `ProxyJump` hops. Wait for the daemon's verified authentication result: it connects the Crew bridge automatically. `connect` can open a bridge using an already available authenticated connection; `disconnect` closes the selected connection. Follow the [SSH hop policy](ssh-hop-policy.md) for known-hosts setup, supported jump syntax and configuration restrictions. SSH transport alone is not a HIPAA compliance determination.

The daemon keeps a connected bridge alive while it is idle and dials it again without a prompt when it drops. After a network failure it tries again after 20 s, 60 s and 180 s, then every 5 minutes for up to an hour, so a connection comes back by itself when the network does. It never dials again after `disconnect`, while a sign-in is pending, or after a failure only a person can fix. If the workspace no longer knows this computer (its host ran `enroll revoke`), the connection stops for good: `connections list` shows it disconnected with `This computer is no longer a member of lab.`, and JSON output carries `"last_error_code": "crew_membership_ended"` beside `last_error`. Nothing reconnects it by itself; `connect` still may, and checks from scratch.

Connection selection is automatic only when exactly one connection is saved. Otherwise pass `--connection` with the connection's name, SSH target or ID. To replace settings or remove the local saved connection:

```bash
biorouter crew --connection lab connections update ./crew-connection.json
biorouter crew --connection lab disconnect
biorouter crew --connection lab connections remove
```

## Create teams and channels, and invite members

```bash
biorouter crew --connection lab profile set 'Alex Kim' --avatar 'AK'
biorouter crew --connection lab teams create 'Analysis Lab'
biorouter crew --connection lab teams list
biorouter crew --connection lab channels create methods --team analysis-lab
biorouter crew --connection lab channels list --team analysis-lab
```

Team creation also creates its `general` channel, classified `restricted` in a private workspace or `public_safe` in a public workspace. Explicit `channels create` defaults to `restricted`; `--classification public-safe` requests that separate classification. Avatars accept short text such as initials or emoji. `profile set` sets the display name people see beside your `@username`; it confers no authority.

Names are unique, and the broker enforces it:

- A team name is unique in the workspace. Letter case, spacing, dashes, dots, invisible characters and look-alike letters from other scripts don't make a second name: `Analysis Lab`, `analysis-lab` and `ANALYSIS_LAB` are the same team.
- A channel name is unique in its team, archived channels included, and `general` is reserved. Channel names are lowercase with dashes, so `channels create 'Data Analysis'` saves `#data-analysis` and says so.
- A refused name reads the same whether or not you can see the team or channel that holds it. After ten such refusals in ten minutes the broker answers `Too many name attempts. Try again later.` Everyone in a team can tell whether a name is taken, so keep identifiers out of names.

Rename with `teams rename analysis-lab 'Analysis Group'` (the team's creator), `channels rename analysis-lab/methods protocols` (the channel's owner) or `workspace rename NEW` (the host). A rename keeps the object's ID, so history, invitations and grants are unaffected.

To invite a member of the workspace to a team you created or a channel you own:

```bash
biorouter crew --connection lab invites create @bob --team analysis-lab
biorouter crew --connection lab invites create @bob --channel analysis-lab/methods

# Bob:
biorouter crew --connection lab invites list
biorouter crew --connection lab invites accept analysis-lab
```

A channel invitation's recipient must already belong to the team. `invites accept` needs no argument when you have one pending invitation; otherwise name the team or `team/channel` it is for. Users can belong to multiple teams.

On a broker that advertises `direct_add_v1`, a team's owner (or the workspace host) can add a member of the workspace straight into the team, its `#general` and any channels of it they own, with nothing for the member to accept: they agreed to take part when they joined the workspace.

```bash
biorouter crew --connection lab members add @bob --team analysis-lab --channel '#methods'
biorouter crew --connection lab members add @bob --channel analysis-lab/methods
```

- The first form answers `Added. @bob can now see #general and #methods.` Without `--team`, each `--channel` adds Bob to a channel you own in a team he already belongs to.
- Only a person's own device can do this, never an agent's grant. The broker checks that you own the team (or each channel) or host the workspace, that `@bob` is still the member you named on this server, and that every channel is in that team; one wrong channel refuses the whole add. Adding someone who is already there is a success that changes nothing, and `--request-id` retries are safe.
- An older broker answers `This workspace's server can't add people directly yet.`; use `invites create` there.

The current channel owner can run `remove-member analysis-lab/methods @bob` (add `--former` for someone who has already left the workspace) or `channels archive analysis-lab/methods`. Ownership transfers require `ownership offer methods @carol` followed by Carol's `ownership accept methods`. **Acceptance removes the previous owner from that channel.**

The host can remove a person from the whole workspace with `enroll revoke @bob`. It revokes their enrollment, devices and agent grants, which is different from removing someone from one channel, so it asks you to type `@bob` again. Where there is no terminal to ask in, pass `--confirm @bob`; `enroll revoke PRINCIPAL_ID` never asks.

These commands name the person the way every list does: `"Bob Lee" (@bob)` when Bob set a display name, and `@crew_frank` alone when the display name is the username, as in `Removed @crew_frank from #methods.`

## Read, post and follow channel messages

```bash
biorouter crew --connection lab history methods --latest --limit 50
biorouter crew --connection lab send methods --text 'The analysis is ready.'
biorouter crew --connection lab send '#methods' --input ./update.txt
biorouter crew --connection lab search analysis-lab/methods 'analysis' --limit 50
biorouter crew --connection lab watch methods
```

`send` prints `Posted to #methods.` History accepts `--before CURSOR` or `--after CURSOR`, and search accepts `--after CURSOR`. Use the opaque cursors returned by Crew without modifying them. `watch` starts from the oldest available messages unless given `--after CURSOR`. The daemon polls, checks channel access and streams authorized pages; the CLI renders that shared stream. Ctrl-C detaches the watcher. `channels mark-read methods` marks the channel read up to its newest message; add a cursor to stop earlier.

Use `--output-format json` for structured single responses. Watch commands emit one JSON value per line with either `json` or `stream-json`. Text output escapes terminal control characters. Content in messages, files and agent output remains untrusted input.

When the daemon ends a watch, text output says why in one sentence, for example `Stopped watching #methods: You no longer have access to this channel.`, or `Stopped watching #methods: This computer isn't a member of this workspace.` when the workspace no longer knows this computer (the words `history` uses for the same refusal). The observer's code (`channel_access_changed`, `scope_changed`, …) is in the JSON error frame, never in the text.

## Transfer attachments or share remote references

```bash
biorouter crew --connection lab --request-id upload-analysis-001 files upload methods ./analysis.txt

TRANSFER_ID='replace-with-transfer-id'
biorouter crew --connection lab files status "$TRANSFER_ID"
biorouter crew --connection lab files watch "$TRANSFER_ID"

# Use blob_id from the completed upload receipt:
BLOB_ID='replace-with-blob-id'
biorouter crew --connection lab send methods --attachment "$BLOB_ID"
biorouter crew --connection lab files download "$BLOB_ID" --output ./downloaded-analysis.txt
```

Uploads and downloads return a transfer receipt before completion; add `--show-ids` or `--output-format json` to see its transfer ID and, once an upload completes, its `blob_id`. Uploading does not automatically post a message; publish the completed `blob_id` with `send --attachment`. Repeat `--attachment` to attach multiple blobs. Add `--overwrite` to download only when you authorize replacing the selected destination. Images and other file types use the same attachment transfer service.

Overwrite approval applies to the file selected at registration. If that file is replaced or changes during the download, publication is refused; a previously absent target cannot be silently replaced. Reselect the destination to approve a changed target when the receipt permits resume. If publication is unconfirmed, inspect the destination first; the daemon will not automatically repeat it.

Choose an existing download destination directory owned by your user and not writable by group or others. The service refuses symlink path components and unsafe destination directories; choose a suitable directory rather than weakening its checks.

`files pending` lists daemon transfer receipts for the selected connection. `files pause TRANSFER_ID` requests a pause; wait until the receipt is inactive before resuming or forgetting it. `files resume TRANSFER_ID PATH` reapproves the upload source or original download destination; retain `--overwrite` when that replacement permission is needed. Ctrl-C on `files watch` leaves the transfer running.

For an incomplete download whose receipt has a `destination_identity`, use `files forget TRANSFER_ID --file ORIGINAL_DESTINATION`. The daemon verifies that selection and removes only its owned partial download before forgetting the receipt; it does not authorize overwriting a published file. For an upload, a completed download or an incomplete download without a destination binding, use `files forget TRANSFER_ID` without `--file`.

A path reference is separate from a transferred attachment:

```bash
biorouter crew --connection lab files reference methods '/project/analysis/results' --label 'Remote results'
biorouter crew --connection lab files show-reference REFERENCE_ID
biorouter crew --connection lab send methods --reference REFERENCE_ID
```

Creating a reference does not upload, download or validate that remote path's contents.

## Run your own agent or grant an existing conversation access

Choose an already configured provider and model allowed by the workspace and your personal privacy mode:

```bash
PROVIDER_NAME='replace-with-configured-provider'
MODEL_NAME='replace-with-approved-model'
biorouter crew --connection lab --request-id analysis-task-001 tasks start methods \
  --input ./task-prompt.txt --provider "$PROVIDER_NAME" --model "$MODEL_NAME" --allow-posting
biorouter crew --connection lab tasks list

RUN_ID='replace-with-run-id'
biorouter crew --connection lab tasks show "$RUN_ID"
biorouter crew --connection lab tasks watch "$RUN_ID"
biorouter crew --connection lab tasks cancel "$RUN_ID"
```

Before tasks or grants, use `privacy show` to inspect `connection_policy_epoch` and `workspace.policy_epoch`. Optional `--expected-policy-epoch` and `--expected-workspace-policy-epoch` bind the observed epochs only for task starts and grants, for example:

```bash
biorouter crew --connection lab --expected-policy-epoch CONNECTION_EPOCH \
  --expected-workspace-policy-epoch WORKSPACE_EPOCH tasks start methods \
  --input ./task-prompt.txt --provider "$PROVIDER_NAME" --model "$MODEL_NAME" --allow-posting
```

Replace the epoch placeholders with the observed integers. Do not reuse stale values after policy changes. These flags do not apply to sends, transfers or cleanup.

A private model that the workspace's institution has not approved is refused before the task starts, in the desktop's words: `gpt-5.5 is approved for ucsf. foreign-lab uses stanford. Choose a model approved for it, or a local model.` (`--output-format json` keeps the daemon's code, `crew_request_refused`.)

A task that reads a file shared in the channel ends its posted result with a line the daemon adds from what the task actually read, for example ``Source: `gina-assay.csv`, shared by Gina Rossi (@crew_gina).`` (several files: `Sources: …`, with `and N more files` past 32). The file name is shown as code, so a name shaped like a link or like the line's own notes stays a name. When two shared files have the same name the agent uses the most recently shared one unless the task names a specific copy; the agent is given the channel's recent files by name, with the newest copy marked, before it reads any. The line tells same-named copies apart: the newest by when it was shared, in the daemon's local time with its offset (``shared by Gina Rossi (@crew_gina) at 2:20 AM UTC-7``, with the day in front when it was not shared the day the result was posted), because a posted line is read long after "newest" stopped being true; an earlier one as `earlier copy`. When the task read only an earlier copy it adds ``A newer copy of `gina-assay.csv` was shared and was not read.`` Two copies shared in the same second are told apart by the order the workspace lists them in. A task that read no shared file ends with `No shared file was read for this result.` instead, so every posted result ends in the daemon's line and a "Source:" line the agent wrote itself is never the last thing in it. Nothing in the agent's reply is removed; a footnote definition in it is shown where it was written, because the channel would otherwise draw it below the daemon's line.

Task starts currently require explicit `--allow-posting` for the destination channel. Repeat `--context-channel analysis-lab/raw-data` to request additional source channels; membership and privacy policy still apply. `tasks list --show-ids` shows run IDs. `tasks watch` follows task status, while channel history contains published activity. Ctrl-C detaches a watcher; cancellation requires `tasks cancel`.

Agents receive task-specific grants and cannot use another user's identity to start or control that user's tasks. Remote file access requires an admitted `remote_root`; remote execution additionally requires `remote_execution: true` in the connection descriptor and an allowed private provider. Agent file paths are relative to that remote work directory. Public-provider grants do not enable these remote file or execution capabilities.

For an existing conversation, open it in the shared daemon and let its current turn finish before granting access. Only a chat saved on this device can be granted access:

```bash
SESSION_ID='replace-with-conversation-session-id'
biorouter crew --connection lab --expected-policy-epoch CONNECTION_EPOCH \
  --expected-workspace-policy-epoch WORKSPACE_EPOCH grants grant "$SESSION_ID" methods
biorouter crew --connection lab grants list
biorouter crew --connection lab context "$SESSION_ID"
biorouter crew --connection lab grants revoke "$SESSION_ID"
```

`grants grant` also accepts repeated `--context-channel`. The daemon validates the conversation's actual provider and origin; a session ID alone does not authorize access. `grants list` shows each grant's kind (a chat, or one of your tasks), the chat's name and when it expires. `context` returns the authorized context manifest.

A grant belongs to the chat it was made for. A later chat that happens to reuse the same session ID does not inherit it. Deleting a granted chat does not revoke its run: the grant stays in `grants list`, still restricting, until you revoke it.

### Revoke access

`grants revoke SESSION_ID` stops a chat, or one of your tasks, from using Crew. It succeeds only when the workspace confirms it:

| Outcome | What the command prints | Exit status |
| --- | --- | --- |
| Confirmed (HTTP 200) | `Access revoked. Chat SESSION_ID can't use Crew until you grant access again, or start a new chat.` For a task session, also `Its task was stopped.` or its status | 0 |
| Stopped here, not confirmed (503 `crew_revocation_unconfirmed`) | `Stopped on this device. The workspace has not confirmed the revocation yet; reconnect and run this command again to confirm.` | 1 |
| Refused | The daemon's reason, for example 404 `crew_grant_not_found`, 409 `crew_grant_other_connection` (the grant belongs to another saved connection; pass that one with `--connection`), 409 `crew_grant_replaced` (a new grant is live), 500 `crew_revocation_not_saved` or 403 `crew_user_action_required` | 1 |

After the 503 the grant is already expired and saved on this device, so this computer's chat can no longer use Crew, but the workspace has not yet confirmed that the run is revoked. Reconnect (`auth` or `connect`) and run `grants revoke` again until it succeeds. With `--output-format json`, every failure prints `{"error": …, "request_id": …, "code": …}`, where `code` is the daemon's code, such as `crew_revocation_unconfirmed`.

A revoked chat stays restricted for good: its next Crew request is refused with `This chat's Crew access was removed. Start a new chat, or grant access again from Crew.` Revocation invalidates the grant; it is not proof that every already-started remote process has terminated.

## Use a terminal conversation through the shared daemon

**Bounded runtime qualification:** `5455ebf9` passes the full repository gate, native/Linux builds, deterministic Crew tool use, zero-dispatch revocation and an installed Qwen3:8b history/context/post workflow. The subsequent merge passes typecheck and the full desktop suite. Native continuation recovery below is committed in `532c3b7d`, independently reviewed and covered by 11 shared-conversation and 16 daemon-client tests. The full repository gate, native and Linux non-test builds, and actual terminal leave/abandon/takeover recovery also pass at that checkpoint. Recovery is unavailable in the older `5455ebf9` pair. The [history mapping](evidence/source-only-history-20260922.md) preserves original evidence scopes; these examples are not a full parity claim.

Create a daemon conversation without sending a model prompt, specifying both provider and model after the `session` subcommand:

```bash
biorouter session --shared-daemon --no-start --create-only \
  --provider "$PROVIDER_NAME" --model "$MODEL_NAME"

# Copy the exact daemon session ID returned above.
SESSION_ID='replace-with-returned-daemon-session-id'
biorouter crew --connection lab --expected-mode private \
  grants grant "$SESSION_ID" methods

# Interactive continuation of that exact daemon conversation:
biorouter session --shared-daemon --no-start --resume --session-id "$SESSION_ID"

# Alternatively, send one prompt to the same daemon conversation:
biorouter run --shared-daemon --no-start --resume --session-id "$SESSION_ID" \
  --text 'Summarize the Crew context that I have authorized for this conversation.'
```

Use the same BioRouter profile throughout, choose an allowed configured provider and model, and finish any active turn before granting access. Creation alone does not grant Crew access. Resume requires the exact session ID; it does not search local sessions. `--no-start` requires an existing daemon. Human approval uses the hidden prompt, or explicit `--approval-key-stdin`; keep that secret out of model prompts and arguments.

The shared adapter uses daemon agent, reply and session services before constructing any local Agent or project bridge. Without `--shared-daemon`, ordinary `run` and `session` remain standalone local conversations; a daemon-issued Crew grant does not supply their process-local manager with the daemon's live SSH connection. A `--no-session` run gets its own session ID namespace, so it can never match a saved chat's grant. Unsupported local-only flags are refused in shared mode. The stream authenticates daemon identity before proof, uses bounded parsing and never automatically resubmits a turn after a transport error.

Ordinary elicitation questions are answered directly through the Proven-only daemon route for that exact session; a desktop redirect is not required. Unsupported approval types receive an explicit refusal. Durable answer and history persistence is implemented and independently reviewed, but the approval, stream and cancellation acceptance matrix remains pending. An unknown question returns a typed no-write refusal; an answer recorded after its waiter ended reports `recorded_not_delivered`, and a persistence failure is distinct. The CLI treats only typed `unknown` as a no-op and never automatically resubmits an answer.

If the exact session has a pending Stop-and-Send continuation, interactive resume
offers `takeover`, `abandon`, or `leave`. Takeover revokes the previous client's
continuation claim; abandonment discards the claim. Neither recovers another
client's unsent draft. Noninteractive commands leave pending claims unchanged.
A settling or changed generation requires another explicit inspection. The next
deliberately submitted turn uses the acquired lease; leaving before submission
releases only that known unused lease. An uncertain submission is never
automatically replayed or abandoned. These choices use the daemon's existing
human-approval and exact-generation checks.

## Privacy, retries and recovery

```bash
biorouter crew --connection lab privacy show
biorouter crew --connection lab privacy set-personal private
# Workspace host only:
biorouter crew --connection lab privacy set-workspace private
```

The accepted mode values are `private` and `public`. Workspace policy, personal mode, channel classification, provider policy and grants jointly restrict operations. A public setting or a `public-safe` channel does not override another restriction or automatically declassify private content. Connection and policy changes invalidate relevant grants and can require reconnection and renewed authorization. Each connection remains a separate workspace scope; cross-channel context is explicitly granted.

For retryable broker mutations, task starts and transfer starts, retain the same `--request-id` and the same operation after an uncertain response. The ID must be 1–128 ASCII letters, digits, underscores or hyphens. Without an explicit ID the CLI generates one. When a mutation was sent and its outcome is unknown (no answer, or a server error), the error ends with a retry hint naming that ID; JSON output always carries `request_id`.

A refusal from the workspace is printed as a sentence, never with the broker's code in front of it: `Only the team's owner or the workspace host can add people to it.`, not `forbidden: Only the team's owner…`, and `This computer isn't a member of this workspace.` for a device the workspace no longer knows. The code stays in JSON output as `broker_code`, beside the daemon's `code`, for scripts and for support. A different intended operation needs a different ID. This is not a blanket transaction mechanism for every command.

Transfer retries reapprove the selected local file and compare the saved operation and file identity. An exact accepted replay returns the existing receipt without launching a second transfer; changed selection, content identity, scope or overwrite authority can be refused. Resume an interrupted transfer using `files resume`, rather than expecting a start replay to resume it.

For recovery, inspect `daemon status`; if the daemon is absent, run `daemon start` first. Unlock an encrypted vault if used, authenticate the connection, then inspect `files pending`, `tasks list` and `grants list`. Do not assume tasks or transfers automatically resumed after a restart. If a task reports interrupted setup, an unknown durable outcome or unconfirmed cancellation, inspect its conversation or channel and retry cancellation or revoke the grant as indicated before deliberately starting another task. A successful local cancellation request alone does not confirm remote process termination.

If daemon identity verification, approval-secret verification, workspace key verification or SSH policy preflight fails, resolve that reported mismatch before reconnecting. Do not replace trusted descriptors, keys or runtime files merely to suppress the refusal. An existing daemon with no human approval proof must be explicitly stopped and relaunched through a trusted launcher.

Implementation references: [CLI arguments](../../../crates/biorouter-cli/src/commands/crew/args.rs), [command routing](../../../crates/biorouter-cli/src/commands/crew/mod.rs), [native daemon client](../../../crates/biorouter-cli/src/daemon_client.rs), [name resolver](../../../crates/biorouter-server/src/routes/crew/names.rs), [invitation and join routes](../../../crates/biorouter-server/src/routes/crew_authentication.rs), [connection schema](../../../crates/biorouter/src/crew/mod.rs), [profile and grant routes](../../../crates/biorouter-server/src/routes/crew_profile.rs), [desktop daemon attachment](../../../ui/desktop/src/biorouterd.ts).

### Synthetic development QA input (bounded launch qualification)

The Electron development shell has a new `--dev-approval-key-stdin` path for explicitly authorized synthetic QA input. It requires an unpackaged app, validated development profile, test driver and shared daemon. Input is bounded, single-use, read through EOF and strictly validated; failures are sanitized. Three automatic no-prompt launches, 46 focused tests, typecheck and the isolated UI build pass; the full gate has a recorded exit 0. Same-profile wrong-proof refusal and correct reopen pass; fresh-profile refusal and full workflow qualification remain pending. This is separate from native CLI `--approval-key-stdin` and does not automate production human proof or SSH/MFA. Keep synthetic values out of arguments, environment, logs and model context.

The current acceptance fixture uses the user-selected existing private `versa_azure` provider with model `gpt-5.5-2026-04-24`. This is a fixture choice, not a new default or a relaxation of Crew provider policy. Earlier Qwen results retain their original scope.

### Institution binding (required work in progress)

New private SSH connection saves require a canonical institution ID in current source. A shared host receives an explicit initial label that cannot change through a privacy toggle; unlabelled legacy hosts allow human collaboration only until labelled. Local providers may serve any institution; institutional providers must match the host’s institution. Grants and retained session data must preserve that binding through connection/workspace epochs and provider changes. The host confirms its immutable shared label with `privacy set-workspace private --institution ucsf` after `workspace bootstrap` and before tasks/grants. Public/private toggles do not erase that label. Source checks and native build pass; live qualification remains incomplete.

Crew-scoped copy, diverge and edit-diverge are refused before a child is created, including scopes retained after revocation or expiry. Start a fresh conversation and explicitly grant Crew context. Ordinary conversation derivation retains the union of institution owners. This source-reviewed restriction is intentional; institution runtime validation remains pending.

## Related documentation

- [Naming design](naming-design.md) — the selector grammar, the resolver's rules and why joining uses an invitation and a device code
- [Broker protocol](protocol-contract.md) — the wire methods these commands call, and the rootless setup checklist
- [UI redesign specification](ui-redesign-spec.md) — the desktop screens that do the same things, including the four places to revoke access
- [SSH hop policy](ssh-hop-policy.md) — host-key trust and jump-host rules for `auth` and `connect`
- [Implementation status](implementation-status.md) — which of these commands have live evidence, and at which revision
- [Implementation plan](implementation-plan.md) — §15 CLI and GUI parity and §16 naming requirements
