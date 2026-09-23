# BioRouter Crew: native CLI and shared desktop daemon

This guide describes the current source implementation in this worktree. It is not a release or end-to-end acceptance claim. The native CLI and desktop Crew panel call the same daemon services for connections, membership, transfers, policy and owned tasks.

Shared profile IPC currently requires Unix: an authenticated, owner-protected Unix socket connects clients to `biorouterd`. Native Windows shared-daemon attachment is not implemented; there is no TCP fallback for the CLI. The remote Crew broker and bridge currently require Linux. A workspace operator must provision that broker before users connect.

## Start the daemon and choose credential storage

For privacy-sensitive actions, add `--expected-mode private` or `--expected-mode public`, for example `biorouter crew --connection CONNECTION_ID --expected-mode private send CHANNEL_ID --text 'Hello'`. The daemon refuses a mismatched saved mode instead of changing it. This optional expectation applies to `send`, `tasks start`, `grants grant`, and file upload, download, and resume selection; omission preserves existing behavior. It does not change the connection's privacy setting or apply to file cleanup.

```sh
biorouter crew --help
biorouter crew daemon status
biorouter crew daemon start
biorouter crew credentials status
```

`daemon status` checks the discovered instance without asking for the human approval secret; it reports an error if no instance is available. Run `daemon start` only when the daemon is absent: it refuses an already running instance. Ordinary Crew commands require the approval secret and can start a missing daemon; add `--no-start` to require an existing instance. Credential commands require an existing daemon.

The CLI and desktop must use the same BioRouter profile to share an instance. On Unix, the desktop normally discovers or starts this profile daemon and asks for its approval secret. Closing the desktop or exiting the CLI leaves the shared daemon running. Explicitly stop it with `biorouter crew daemon stop`; that affects every client attached to the instance. An explicit desktop shared-daemon opt-out or an external backend is a different deployment mode.

In the desktop Crew panel, select the same saved connection, use **Authenticate** for SSH prompts, and select a team/channel for messages and tasks. **Crew credentials** manages the same keyring/vault backend used by the CLI. Desktop file pickers and CLI file paths both register selections with the daemon's transfer service; switching clients does not create a separate workspace or transfer engine.

There are three separate credentials:

| Credential | Purpose and input |
| --- | --- |
| Human approval secret | Authorizes human operations in this daemon. Choose and retain it separately when starting a new instance; supply the same secret when attaching. It must contain 32–4096 printable ASCII characters without spaces. It is not recovered from daemon discovery metadata or desktop settings. |
| Optional encrypted-vault passphrase | Unlocks this profile's Crew device credentials. It must differ from the approval secret; the vault accepts 1–1024 UTF-8 bytes. The OS keyring is the default credential backend. |
| SSH credentials and MFA responses | Enter only in the native SSH authentication terminal. They are not Crew messages, agent prompts, enrollment tokens or vault passphrases. |

To use an encrypted vault, initialize it **before preparing identities or saving connections in a fresh Crew profile**. Existing keyring identities are not migrated by this command:

```sh
biorouter crew credentials init
biorouter crew credentials lock
biorouter crew credentials unlock
```

The default secret prompts hide input. `--approval-key-stdin` explicitly reads exactly the first stdin line as the approval secret. For `credentials init` or `credentials unlock`, the second line supplies the distinct vault passphrase. For `enroll accept --token-stdin`, the enrollment token is the next line after approval. With `send --input -` or `connections save -`, the remaining stdin content is the message or JSON descriptor. Supply such input through a trusted secret-input pipe; do not put secrets in command arguments, shell history, environment variables or connection JSON. Enrollment also supports an explicit `--token-fd FD`. Native `auth` still requires an interactive terminal, so use its normal hidden approval prompt.

## Install the remote executable before connecting

Every connecting ordinary Unix account on the Linux SSH host needs its own executable at **`~/.local/bin/biorouter-crew`**. The daemon invokes that exact path for `bridge --stdio`; an executable only in `/usr/local/bin` or `/usr/bin`, or available only through `PATH`, does not satisfy this prerequisite. The account hosting the broker needs it too.

Obtain and verify a `biorouter-crew` artifact qualified for the target Linux architecture and runtime. In each connecting account's own SSH session, install the verified file without administrator privileges:

```sh
# Run on the remote Linux host as each connecting ordinary user.
VERIFIED_BINARY='/replace/with/path/to/verified/linux/biorouter-crew'
mkdir -p "$HOME/.local/bin"
install -m 0755 "$VERIFIED_BINARY" "$HOME/.local/bin/biorouter-crew"
"$HOME/.local/bin/biorouter-crew" --version
```

Only the initial workspace owner starts the shared broker; colleagues run bridges under their own accounts and do not start another broker for that workspace. After `biorouter crew connections prepare`, the owner uses the returned public device key with the remote `biorouter-crew start --state-dir PATH --bootstrap-key HEX` command, then verifies `status --state-dir PATH`. See the [rootless setup checklist](protocol-contract.md#rootless-setup-checklist) for the complete commands and trusted connection details. The local `biorouter crew --connection CONNECTION_ID workspace bootstrap` command performs signed `auth.bootstrap` after authentication; it does not install or launch the remote broker.

Use qualified local persistent storage and an institution-approved SSH route. Check [Linux artifact portability](linux-portability.md) and [institutional SSH requirements](institutional-ssh-compatibility.md) before deployment; version output alone does not qualify kernel, filesystem, confinement or institution policy. Do not infer compatibility with an older cluster runtime from the current ARM64 test artifact's GLIBC 2.39 requirement.

## Save a verified connection, then authenticate

Prepare a device identity first. `connections prepare` and `enroll prepare` are aliases for the same operation:

```sh
biorouter crew connections prepare
biorouter crew connections save ./crew-connection.json
biorouter crew connections list
```

Build `crew-connection.json` from verified workspace information supplied by its operator. It is a JSON object with these fields; it contains no device private key or password:

| Field | Value |
| --- | --- |
| `preparation_id` | The ID from `connections prepare`, to use the device public key already shared for enrollment. Use only when saving a new connection. |
| `name` | A display name. |
| `ssh_target` | Your SSH host alias or `user@host`, using your own remote account. |
| `socket_path` | The broker's verified absolute remote socket path. |
| `owner_uid` | Numeric Unix UID of the remote broker owner; this is not necessarily your own SSH UID. |
| `workspace_id` | The operator's verified workspace UUID. |
| `workspace_public_key` | The broker's verified 32-byte public key encoded as 64 hexadecimal characters. This is different from your prepared device public key. |
| `mode` | `private` or `public`; omission defaults to `private`. |

Optional fields are `port` (number), `identity_file` (absolute local path), `proxy_jump` (SSH jump route), `remote_root` (remote work directory), `remote_execution` (boolean, default `false`) and `cluster_connection_id` (existing cluster connection ID). Unknown fields are rejected. `connections show` includes read-only state, so its entire output is not a valid save/update descriptor. For updates, omit `preparation_id` and supply the complete editable descriptor.

In subsequent examples, replace the uppercase placeholder values with IDs returned by Crew. Names alone do not select a connection, channel, team or run:

```sh
CONNECTION_ID='replace-with-saved-connection-id'
biorouter crew --connection "$CONNECTION_ID" connections show
biorouter crew --connection "$CONNECTION_ID" auth
```

The saved connection ID is `id` in the response. Team creation returns `team.id` and `channel.id`; explicit channel creation returns `id`. Transfer responses use `id`, and task starts use `run_id`. These fields are available with `--output-format json`.

`auth` opens the daemon-owned native SSH terminal for host authentication, passwords and MFA, including admitted `ProxyJump` hops. Wait for the daemon's verified authentication result: it connects the Crew bridge automatically. `connect` can open a bridge using an already available authenticated connection; `disconnect` closes the selected connection. Follow the [SSH hop policy](ssh-hop-policy.md) for known-hosts setup, supported jump syntax and configuration restrictions. SSH transport alone is not a HIPAA compliance determination.

Connection selection is automatic only when exactly one connection is saved. Otherwise pass `--connection`. To replace settings or remove the local saved connection:

```sh
biorouter crew --connection "$CONNECTION_ID" connections update ./crew-connection.json
biorouter crew --connection "$CONNECTION_ID" disconnect
biorouter crew --connection "$CONNECTION_ID" connections remove
```

## Enroll people and create shared spaces

Only the initial operator whose device key matches the broker bootstrap key uses `workspace bootstrap`. Colleagues share their prepared **public** device key and remote numeric Unix UID with that operator. The operator issues an enrollment invitation, then the colleague accepts its token through the hidden prompt:

```sh
# Initial operator only, after authentication:
biorouter crew --connection "$CONNECTION_ID" workspace bootstrap

# Operator: substitute the colleague's actual numeric UID and public key.
biorouter crew --connection "$CONNECTION_ID" enroll invite --uid 12345 --public-key 'REPLACE_WITH_DEVICE_PUBLIC_KEY_HEX'

# Invited colleague, using their own saved connection and SSH account:
biorouter crew --connection "$CONNECTION_ID" enroll accept
biorouter crew --connection "$CONNECTION_ID" workspace show
biorouter crew --connection "$CONNECTION_ID" members
```

Deliver the returned enrollment token through an approved private channel. For an additional device belonging to an existing principal, the operator adds `--existing-principal PRINCIPAL_ID`. `enroll revoke PRINCIPAL_ID` revokes that principal's enrollment, devices and grants; it is different from removing someone from one channel. The authenticated remote account and enrolled key establish identity; profile nicknames do not change that authority.

```sh
biorouter crew --connection "$CONNECTION_ID" profile set 'Alex' --avatar 'A'
biorouter crew --connection "$CONNECTION_ID" teams create 'Analysis team'
biorouter crew --connection "$CONNECTION_ID" teams list

TEAM_ID='replace-with-team-id'
biorouter crew --connection "$CONNECTION_ID" channels create 'analysis' --team "$TEAM_ID"
biorouter crew --connection "$CONNECTION_ID" channels list --team "$TEAM_ID"
```

Team creation also creates its `general` channel, classified `restricted` in a private workspace or `public_safe` in a public workspace. Explicit `channels create` defaults to `restricted`; `--classification public-safe` requests that separate classification. Avatars currently accept short text such as initials or emoji.

The team owner can invite a principal with `invites create PRINCIPAL_ID --team TEAM_ID`. A channel owner can use `invites create PRINCIPAL_ID --channel CHANNEL_ID`; the recipient must also satisfy team membership. The recipient runs `invites list`, then `invites accept INVITATION_ID`. Users can belong to multiple teams.

The current channel owner can run `remove-member CHANNEL_ID PRINCIPAL_ID` or `channels archive CHANNEL_ID`. Ownership transfers require `ownership offer CHANNEL_ID SUCCESSOR_ID` followed by the successor's `ownership accept CHANNEL_ID`. **Acceptance removes the previous owner from that channel.** These commands all take the same global `--connection` selection as the examples above.

## Read, post and follow channel messages

```sh
CHANNEL_ID='replace-with-channel-id'
biorouter crew --connection "$CONNECTION_ID" history "$CHANNEL_ID" --latest --limit 50
biorouter crew --connection "$CONNECTION_ID" send "$CHANNEL_ID" --text 'The analysis is ready.'
biorouter crew --connection "$CONNECTION_ID" send "$CHANNEL_ID" --input ./update.txt
biorouter crew --connection "$CONNECTION_ID" search "$CHANNEL_ID" 'analysis' --limit 50
biorouter crew --connection "$CONNECTION_ID" watch "$CHANNEL_ID"
```

History accepts `--before CURSOR` or `--after CURSOR`, and search accepts `--after CURSOR`. Use the opaque cursors returned by Crew without modifying them. `watch` starts from the oldest available messages unless given `--after CURSOR`. The daemon polls, checks channel access and streams authorized pages; the CLI renders that shared stream. Ctrl-C detaches the watcher. `channels mark-read CHANNEL_ID CURSOR` advances your read marker.

Use `--output-format json` for structured single responses. Watch commands emit one JSON value per line with either `json` or `stream-json`. Text output escapes terminal control characters. Content in messages, files and agent output remains untrusted input.

## Transfer attachments or share remote references

```sh
biorouter crew --connection "$CONNECTION_ID" --request-id upload-analysis-001 files upload "$CHANNEL_ID" ./analysis.txt

TRANSFER_ID='replace-with-transfer-id'
biorouter crew --connection "$CONNECTION_ID" files status "$TRANSFER_ID"
biorouter crew --connection "$CONNECTION_ID" files watch "$TRANSFER_ID"

# Use blob_id from the completed upload receipt:
BLOB_ID='replace-with-blob-id'
biorouter crew --connection "$CONNECTION_ID" send "$CHANNEL_ID" --attachment "$BLOB_ID"
biorouter crew --connection "$CONNECTION_ID" files download "$BLOB_ID" --output ./downloaded-analysis.txt
```

Uploads and downloads return a transfer receipt before completion. Uploading does not automatically post a message; publish the completed `blob_id` with `send --attachment`. Repeat `--attachment` to attach multiple blobs. Add `--overwrite` to download only when you authorize replacing the selected destination. Images and other file types use the same attachment transfer service.

Overwrite approval applies to the file selected at registration. If that file is replaced or changes during the download, publication is refused; a previously absent target cannot be silently replaced. Reselect the destination to approve a changed target when the receipt permits resume. If publication is unconfirmed, inspect the destination first; the daemon will not automatically repeat it.

Choose an existing download destination directory owned by your user and not writable by group or others. The service refuses symlink path components and unsafe destination directories; choose a suitable directory rather than weakening its checks.

`files pending` lists daemon transfer receipts for the selected connection. `files pause TRANSFER_ID` requests a pause; wait until the receipt is inactive before resuming or forgetting it. `files resume TRANSFER_ID PATH` reapproves the upload source or original download destination; retain `--overwrite` when that replacement permission is needed. Ctrl-C on `files watch` leaves the transfer running.

For an incomplete download whose receipt has a `destination_identity`, use `files forget TRANSFER_ID --file ORIGINAL_DESTINATION`. The daemon verifies that selection and removes only its owned partial download before forgetting the receipt; it does not authorize overwriting a published file. For an upload, a completed download or an incomplete download without a destination binding, use `files forget TRANSFER_ID` without `--file`.

A path reference is separate from a transferred attachment:

```sh
biorouter crew --connection "$CONNECTION_ID" files reference "$CHANNEL_ID" '/project/analysis/results' --label 'Remote results'
biorouter crew --connection "$CONNECTION_ID" files show-reference REFERENCE_ID
biorouter crew --connection "$CONNECTION_ID" send "$CHANNEL_ID" --reference REFERENCE_ID
```

Creating a reference does not upload, download or validate that remote path's contents.

## Run your own agent or grant an existing conversation access

Choose an already configured provider/model allowed by the workspace and your personal privacy mode:

```sh
PROVIDER_NAME='replace-with-configured-provider'
MODEL_NAME='replace-with-approved-model'
biorouter crew --connection "$CONNECTION_ID" --request-id analysis-task-001 tasks start "$CHANNEL_ID" \
  --input ./task-prompt.txt --provider "$PROVIDER_NAME" --model "$MODEL_NAME" --allow-posting
biorouter crew --connection "$CONNECTION_ID" tasks list

RUN_ID='replace-with-run-id'
biorouter crew --connection "$CONNECTION_ID" tasks show "$RUN_ID"
biorouter crew --connection "$CONNECTION_ID" tasks watch "$RUN_ID"
biorouter crew --connection "$CONNECTION_ID" tasks cancel "$RUN_ID"
```

Task starts currently require explicit `--allow-posting` for the destination channel. Repeat `--context-channel CHANNEL_ID` to request additional source channels; membership and privacy policy still apply. `tasks watch` follows task status, while channel history contains published activity. Ctrl-C detaches a watcher; cancellation requires `tasks cancel`.

Agents receive task-specific grants and cannot use another user's identity to start or control that user's tasks. Remote file access requires an admitted `remote_root`; remote execution additionally requires `remote_execution: true` in the connection descriptor and an allowed private provider. Agent file paths are relative to that remote work directory. Public-provider grants do not enable these remote file/execution capabilities.

For an existing conversation, open it in the shared daemon and let its current turn finish before granting access:

```sh
SESSION_ID='replace-with-conversation-session-id'
biorouter crew --connection "$CONNECTION_ID" grants grant "$SESSION_ID" "$CHANNEL_ID"
biorouter crew --connection "$CONNECTION_ID" grants list
biorouter crew --connection "$CONNECTION_ID" context "$SESSION_ID"
biorouter crew --connection "$CONNECTION_ID" grants revoke "$SESSION_ID"
```

`grants grant` also accepts repeated `--context-channel`. The daemon validates the conversation's actual provider and origin; a session ID alone does not authorize access. `context` returns the authorized context manifest. Revocation invalidates the grant; it is not proof that every already-started remote process has terminated.

## Use a terminal conversation through the shared daemon

**Bounded runtime qualification:** `5455ebf9` passes the full repository gate, native/Linux builds, deterministic Crew tool use, zero-dispatch revocation and an installed Qwen3:8b history/context/post workflow. The subsequent merge passes typecheck and the full desktop suite. Native continuation recovery below is committed in `532c3b7d`, independently reviewed and covered by 11 shared-conversation and 16 daemon-client tests. The full repository gate, native and Linux non-test builds, and actual terminal leave/abandon/takeover recovery also pass at that checkpoint. Recovery is unavailable in the older `5455ebf9` pair. The [history mapping](evidence/source-only-history-20260922.md) preserves original evidence scopes; these examples are not a full parity claim.

Create a daemon conversation without sending a model prompt, specifying both provider and model after the `session` subcommand:

```sh
biorouter session --shared-daemon --no-start --create-only \
  --provider "$PROVIDER_NAME" --model "$MODEL_NAME"

# Copy the exact daemon session ID returned above.
SESSION_ID='replace-with-returned-daemon-session-id'
biorouter crew --connection "$CONNECTION_ID" --expected-mode private \
  grants grant "$SESSION_ID" "$CHANNEL_ID"

# Interactive continuation of that exact daemon conversation:
biorouter session --shared-daemon --no-start --resume --session-id "$SESSION_ID"

# Alternatively, send one prompt to the same daemon conversation:
biorouter run --shared-daemon --no-start --resume --session-id "$SESSION_ID" \
  --text 'Summarize the Crew context that I have authorized for this conversation.'
```

Use the same BioRouter profile throughout, choose an allowed configured provider/model, and finish any active turn before granting access. Creation alone does not grant Crew access. Resume requires the exact session ID; it does not search local sessions. `--no-start` requires an existing daemon. Human approval uses the hidden prompt, or explicit `--approval-key-stdin`; keep that secret out of model prompts and arguments.

The shared adapter uses daemon agent, reply and session services before constructing any local Agent or project bridge. Without `--shared-daemon`, ordinary `run` and `session` remain standalone local conversations; a daemon-issued Crew grant does not supply their process-local manager with the daemon's live SSH connection. Unsupported local-only flags are refused in shared mode. The stream authenticates daemon identity before proof, uses bounded parsing and never automatically resubmits a turn after a transport error.

Ordinary elicitation questions are answered directly through the Proven-only daemon route for that exact session; a desktop redirect is not required. Unsupported approval types receive an explicit refusal. Durable answer/history persistence is implemented and independently reviewed, but the approval/stream/cancellation acceptance matrix remains pending. An unknown question returns a typed no-write refusal; an answer recorded after its waiter ended reports `recorded_not_delivered`, and a persistence failure is distinct. The CLI treats only typed `unknown` as a no-op and never automatically resubmits an answer.

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

```sh
biorouter crew --connection "$CONNECTION_ID" privacy show
biorouter crew --connection "$CONNECTION_ID" privacy set-personal private
# Workspace manager only:
biorouter crew --connection "$CONNECTION_ID" privacy set-workspace private
```

The accepted mode values are `private` and `public`. Workspace policy, personal mode, channel classification, provider policy and grants jointly restrict operations. A public setting or a `public-safe` channel does not override another restriction or automatically declassify private content. Connection and policy changes invalidate relevant grants and can require reconnection and renewed authorization. Each connection remains a separate workspace scope; cross-channel context is explicitly granted.

For retryable broker mutations, task starts and transfer starts, retain the same `--request-id` and the same operation after an uncertain response. The ID must be 1–128 ASCII letters, digits, underscores or hyphens. Without an explicit ID the CLI generates one and includes it in operation errors. A different intended operation needs a different ID. This is not a blanket transaction mechanism for every command.

Transfer retries reapprove the selected local file and compare the saved operation and file identity. An exact accepted replay returns the existing receipt without launching a second transfer; changed selection, content identity, scope or overwrite authority can be refused. Resume an interrupted transfer using `files resume`, rather than expecting a start replay to resume it.

For recovery, inspect `daemon status`; if the daemon is absent, run `daemon start` first. Unlock an encrypted vault if used, authenticate the connection, then inspect `files pending`, `tasks list` and `grants list`. Do not assume tasks or transfers automatically resumed after a restart. If a task reports interrupted setup, an unknown durable outcome or unconfirmed cancellation, inspect its conversation/channel and retry cancellation or revoke the grant as indicated before deliberately starting another task. A successful local cancellation request alone does not confirm remote process termination.

If daemon identity verification, approval-secret verification, workspace key verification or SSH policy preflight fails, resolve that reported mismatch before reconnecting. Do not replace trusted descriptors, keys or runtime files merely to suppress the refusal. An existing daemon with no human approval proof must be explicitly stopped and relaunched through a trusted launcher.

Implementation references: [CLI arguments](../../../crates/biorouter-cli/src/commands/crew/args.rs), [command routing](../../../crates/biorouter-cli/src/commands/crew/mod.rs), [native daemon client](../../../crates/biorouter-cli/src/daemon_client.rs), [connection schema](../../../crates/biorouter/src/crew/mod.rs), [profile/grant routes](../../../crates/biorouter-server/src/routes/crew_profile.rs), [desktop daemon attachment](../../../ui/desktop/src/biorouterd.ts).

### Synthetic development QA input (bounded launch qualification)

The Electron development shell has a new `--dev-approval-key-stdin` path for explicitly authorized synthetic QA input. It requires an unpackaged app, validated development profile, test driver and shared daemon. Input is bounded, single-use, read through EOF and strictly validated; failures are sanitized. Three automatic no-prompt launches, 46 focused tests, typecheck and the isolated UI build pass; the full gate also emitted all success outputs, while full workflow qualification remains pending. This is separate from native CLI `--approval-key-stdin` and does not automate production human proof or SSH/MFA. Keep synthetic values out of arguments, environment, logs and model context.
