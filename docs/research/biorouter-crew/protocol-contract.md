# Crew broker protocol v1

Implementation: `crates/biorouter-crew`. The production broker and SSH bridge require Linux; unsupported kernel facilities and unqualified network storage fail explicitly. Product code has been formatted; test/build evidence belongs to the independent Luna lane, not this contract.

## Transport and identity

Run an ordinary-user broker with `biorouter-crew serve --state-dir "$HOME/crew-workspace" --bootstrap-key HEX_ED25519_PUBLIC_KEY`. The state directory's parent must exist. State is owner-only, the initial mode is Private, and the supplied bootstrap key must be the intended host desktop device key. `start` launches the same executable and returns a starting PID; `status` verifies a live matching workspace; `stop` verifies the matching socket peer, workspace and pidfd before sending SIGTERM. Stop waits up to five seconds on the verified pidfd and reports `stopped`; a false value means signal delivered but termination not yet confirmed. No sudo or account/group creation is used.

The broker prints `{pid,socket,workspace_id,host_uid,protocol}`. Share the socket path, workspace ID and independently verified workspace-key fingerprint with enrolled colleagues. The runtime basename is selected randomly once, journaled, and reused on the same node across restarts and temporary-directory loss. Existing directory/socket ownership, types and modes are checked; an unexpected occupant or live listener causes an explicit refusal. The directory is node-local and 0711; its socket is 0666, while every request remains authenticated by kernel peer UID and device signature or a limited worker grant. All persistent files and directories are 0600/0700. The hosting UID is trusted: file modes do not isolate hostile code executing under that same UID.

The fixed remote command is `biorouter-crew bridge --stdio --socket PATH --owner-uid UID --workspace-id UUID`. The bridge validates socket/directory ownership, broker SO_PEERCRED and workspace ID before forwarding. The desktop additionally pins and verifies the workspace key via `hello`, before sending enrollment or any protected request. Native OpenSSH verifies remote host identity independently. The broker never accepts a caller-provided username or UID as authority.

Each frame is a newline-terminated JSON object, maximum 1 MiB:

```json
{"version":1,"id":"client-request-id","method":"messages.history","params":{"channel_id":"uuid"},"auth":{"device_id":"sha256-hex","nonce":"challenge","signature":"ed25519-signature-hex"}}
```

Responses are `{id,result}` or `{id,error:{code,message}}`. Unknown operations return `unsupported`; authorization, privacy and expiry failures are explicit errors, never empty-history success. Parameter strings are bounded, IDs opaque, binary file content hexadecimal. At most 256 live connections, with at most eight per kernel UID, are admitted. Each has bounded framing, a write timeout and a five-minute idle timeout. Reconnect performs fresh identity proof and challenge signing.

`hello {challenge_nonce}` returns protocol, workspace_id, host_uid, mode, policy_epoch, workspace_public_key (raw Ed25519 public key hex), workspace_key_fingerprint (SHA256 of raw public key), node_id (stable machine identity digest), echoed challenge_nonce, signature, capabilities and unsupported facilities. Verify signature over UTF-8 JSON serialization of `[workspace_id,host_uid,challenge_nonce,workspace_public_key,node_id]`. Supply a fresh unpredictable nonce. Changed keys or workspace/host identities require explicit trusted recovery; do not silently repin.

`auth.challenge {device_id}` returns `{nonce,workspace_id,uid,expires_at}`. Device ID is SHA256 of the raw 32-byte device public key. Sign UTF-8 JSON serialization of `[workspace_id,uid,nonce,method,params]`, recursively ordering object keys lexically. The crate's `signing_payload` defines these bytes. Nonces are single-use, bound to the live socket and UID, and expire after 60 seconds. The human key stays in the local trusted device signer; model tools and remote workers never receive it.

`auth.bootstrap {public_key}` is a signed request admitted only once, for host UID and the key pinned at broker creation. `enrollment.invite {uid,public_key,idempotency_key}` is host-device signed and creates a one-hour one-use invitation pinned to both UID and intended device key. `auth.enroll {public_key,invitation}` requires proof by that same intended key and kernel UID. Both enrollment operations return `{principal,device_id,workspace}`. Additional devices require another manager-approved key-bound invitation with `existing_principal_id` matching the active principal for that UID. A changed account name is refused until the old principal is offboarded. Acceptance rechecks the exact intended principal. Offboarding and then enrolling creates a fresh UUID without prior memberships. The kernel cannot distinguish a recycled UID with an unchanged username; the manager must explicitly choose add-device versus offboard/new-account intent. `enrollment.revoke {principal_id,idempotency_key}` disables the principal, devices and relevant pending enrollment, and invalidates existing grants. Enrollment itself does not grant team/channel membership or SSH login access.

## Human collaboration

Every mutation below requires `idempotency_key` in params, signed together with the operation. Keys are scoped to human principal or worker run. Repeating identical bytes returns the same authorized operation result; reusing a key for different bytes fails `conflict`. Cached results are rechecked and projected through current visibility rules before return. Read operations do not require keys.

| Method | Parameters besides mutation key | Result |
|---|---|---|
| `workspace.snapshot` | none | `{workspace,protected_channel_ids,actor,principals,teams,channels,invitations,runs}` filtered to actor-visible objects |
| `channel.read` | channel_id, sequence (opaque visible message token) | Durable monotonic read position; snapshot includes authorized `read_positions` and `unread` maps |
| `profile.update` | nickname, optional avatar | Principal; avatar is at most 12 printable Unicode characters, rendered only as text |
| `team.create` | name | `{team,channel}` with automatic general channel |
| `channel.create` | team_id, name, classification | Channel; classification `public_safe` or `restricted`, default restricted |
| `invitation.create` | kind (`team`/`channel`), target_id, principal_id | Invitation; target must accept |
| `invitation.accept` | invitation_id | `{accepted:true}` |
| `channel.archive` | channel_id | Channel; history retained, future writes denied |
| `channel.transfer` | channel_id, successor_id | Channel with pending_owner; successor must already belong |
| `transfer.accept` | channel_id | Channel; owner changes, immutable created_by retained |
| `membership.revoke` | channel_id, principal_id | Channel; current owner required, cannot remove self |
| `message.post` | channel_id, body, optional attachments, personal_mode | Message; mode missing/unknown conservatively marks contribution restricted |
| `messages.history` | channel_id, optional after, before, limit, latest | `{messages,cursor}`; limit defaults 100, maximum 200 |
| `messages.search` | channel_id, query, optional after, before, limit, latest | same, authorized literal case-insensitive text search |
| `policy.set` | mode (`private`/`public`) | Workspace; host-device only, increments policy epoch |

Persisted data types live in `src/lib.rs`; wire projections replace internal message sequence counters with opaque tokens. Principal includes id, verified uid, canonical username, nickname, textual avatar, active. Channel includes id, team_id, name, created_by, owner_id, members, archived, classification, pending_owner. Message includes id, sequence, channel_id, actor_id, optional run_id, body, created_at (Unix seconds), restricted, source_channels, attachments, optional status. A team's general room adds accepted team members automatically. Restricted channels require their current owner's invitation. Transfers remove the previous owner's implicit membership (a separate successor invitation is needed to rejoin) and invalidate old-owner invitations and all worker grants through the global policy epoch. Joining a channel never grants access to another channel's derived content.

## Attachments

`blob.begin {channel_id,name,media_type,size,sha256,personal_mode,idempotency_key}` returns Blob with opaque id, owner_id, channel_id, display name, media_type, declared size/digest, offset, complete, restricted, source_channels. Names are display text only and never filesystem paths. Limit: 1 GiB per blob, 10 GiB declared total workspace bytes, 10,000 blobs.

`blob.chunk {blob_id,offset,data_hex,idempotency_key}` accepts at most 262144 decoded bytes at exactly the committed offset and returns updated Blob. It fsyncs bytes before journaling the offset. Duplicate retry keys return the prior result; after uncertain failure, query `blob.status {blob_id}` and resume from its committed offset. Trailing uncommitted bytes are overwritten by the next chunk. `blob.finish {blob_id,idempotency_key}` requires exact declared size and SHA256, fsyncs, then commits completion. Incomplete files are never downloadable. `blob.read {blob_id,offset}` returns `{blob,offset,data_hex,next_offset,complete}` with at most one chunk, rechecking membership and source restrictions each time. Completed attachments may be referenced in a message only in their destination channel with their restrictions preserved.

## Owned agent grants and provenance

Human-signed `run.create {channel_id,source_channels,provider_policy_id,public_provider,personal_mode,expires_in,idempotency_key,expected_workspace_policy_epoch,expected_protected_context,workspace_institution_id,connection_institution_id,provider_affiliation}` returns `{run,credential}`. The trusted local controller, not model-supplied parameters, supplies the attested endpoint policy and canonical personal mode. The broker cannot independently classify an external endpoint. Source channels include the destination; at most 20 channels, grant lifetime 1–3600 seconds. Public admission requires Public workspace and personal mode, public-safe channels, and no retained restricted messages in the selected sources. Private mode never declassifies old messages or blobs.

Workspace `institution_id` is nullable for legacy/unlabelled state. An authorized human host confirms the first label through `policy.set`; a non-null label cannot be cleared or changed, including through public/private mode changes. Canonical IDs are 1–64 lowercase ASCII letters, digits, underscores or hyphens, beginning with a letter or digit. Policy mutation increments the workspace policy epoch and revokes existing runs. `workspace.snapshot.protected_channel_ids` contains only channels the actor may read, derived from current restricted channel/content/reference state; it is authoritative rather than a client classification guess.

`run.create` requires the current `expected_workspace_policy_epoch`, a boolean `expected_protected_context`, and both institution keys even when their value is null. `provider_affiliation` is a tagged object: `{"kind":"local"}`, `{"kind":"institutions","institution_ids":["ucsf"]}`, or `{"kind":"unstated"}`. The daemon supplies affiliation from the resolved provider instance, never model or untrusted caller metadata. Local affiliation can serve any institution; institutional affiliation must include the required institution. Protected grants require an established workspace label and compatible connection/provider scope. Unlabelled legacy workspaces remain usable for human collaboration, but legacy private agent grants are denied until labelled. The new client requires the upgraded broker contract; there is no silent fallback that omits these checks, and this document does not introduce a new envelope protocol version.

The persisted `protected_context` records admission state. Worker authorization re-evaluates current protection and institution policy: a false-to-true protection transition invalidates the grant before worker responses, requiring refreshed state and a fresh human grant. Snapshot/preflight values do not authorize returning content after a policy race.

Run persists id, owner_id, channel_id, source_channels, provider_policy_id, protected_context, provider_affiliation, workspace_institution_id, connection_institution_id, public_provider, personal_mode, policy_epoch, expires_at and revoked. Only its owner can create or revoke it. `run.revoke {run_id,idempotency_key}` is human-signed. Worker requests use envelope `credential` instead of `auth`. The opaque token binds the run and kernel UID, and grants only messages.history/search, context.manifest, blob.status/read/begin/chunk/finish, run.remote_scope and run.project. Workers cannot bootstrap, enroll, change members/policy, mint runs or sign approvals.

`context.manifest {}` returns `{run_id,policy_epoch,source_channels,messages,restricted}`, at most the newest 200 authorized source messages. The worker must request this immediately before dispatch as well as on retrieval. Every worker action rechecks active enrollment, UID, grant expiry/revocation, policy epoch, channel membership and output-channel status. Source ACLs and public restrictions filter before snippets/body delivery. Unknown or unavailable source channels are denied.

`run.project {body,attachments?,status?,idempotency_key}` writes only to the granted destination. Status is progress/completed/failed/cancelled. The projection inherits all selected source-channel dependencies and retained restrictions. Every future human or worker reader must satisfy all source memberships; future membership in the destination does not declassify the projection. Terminal status revokes the grant. Full owner sessions, hidden reasoning, credential prompts and raw provider internals must never be passed to this projection API.

## Durability and explicit limits

One exclusive flock writer owns a private checksummed, sequence-checked, chained JSONL journal. Each v2 commit records actor, operation, timestamp and typed Set/Remove/Append deltas over the authoritative state, including policy/membership and idempotency changes. The initial record contains a complete state; legacy v1 full-state records remain replayable with their original checksums. Journal fsync precedes success acknowledgment. A write failure poisons mutation acceptance until restart. Replay rejects interior corruption, bad checksums and noncontinuous sequences; an incomplete final line is copied to a private quarantine file and truncated while exclusively locked. Attachment fsync precedes committed offset/completion. Source restrictions survive restart and policy transitions.

Delta records append only new messages and changed map entries rather than copying accumulated history. Replay streams bounded records, checks every retained checksum and sequence, and does not delete or skip old audit records. The full logical state currently has a 16 MiB limit and the retained journal a 1 GiB limit; quota errors preserve readable history and refuse further mutations; no supported in-place pruning exists. Independent stress/fault tests remain required before claiming the planned 50-user soak. Network filesystems including NFS/SMB are refused pending qualified fencing/durability support. Physical deletion, retention compaction, automatic host failover, release/declassification, cross-workspace retrieval, arbitrary remote shell/filesystem operations and binary/avatar previews are not supplied by this broker. The UI must display these limits rather than manufacture success. A remote owner worker is a separate controller/helper capability with its own admitted policy; `run.create` does not execute a process as the broker host.


### Own-UID remote helper integration

The signed `run.create` may additionally include `remote_root` (absolute, narrower than `/`) and `remote_execution` (default false). Remote filesystem scope is denied for public-provider runs. A worker can request `run.remote_scope` only with its limited credential; the result is `{root,allow_execute,public_provider,run_id,policy_epoch,expires_at}`. The SSH bridge intercepts `remote.*`, authorizes against that operation under the bridge's actual kernel UID, invokes the separate narrow helper under the same UID, then rechecks the grant before returning output. Revocation during an operation withholds output and reports that an effect may already have occurred. The helper defines supported operations and confinement requirements; broker admission alone does not establish shell isolation.

The journal refuses a delta record or logical state beyond 16 MiB, and total retained audit beyond 1 GiB. All audit records remain retained; no rotation or lossy compaction is performed. Replay keeps one bounded record plus current state in memory, rather than loading the complete journal.


Worker attachment uploads bind `Blob.run_id` to the admitted run, fix the destination to that run's channel, and inherit every selected source channel and restriction. Chunk and finish refuse human uploads and other runs' uploads, including those of the same owner. Remote-derived uploads and projections remain restricted even in a Public workspace. Exact retries of an already committed terminal projection can recover their acknowledgment after terminal revocation, but only while UID, active identity, expiry, policy epoch and all memberships still match; no new operation is authorized.

`messages.history` and `messages.search` accept `latest:true` to return the newest authorized window in ascending order. The default remains forward cursor paging. `channel.read` resolves an authorized opaque message token to a monotonically increasing internal read watermark. Snapshot exposes a visible message token or `null` for each read position, plus counts of authorized newer messages from other principals. There is no push/OS notification facility yet.


### Large dataset references

`reference.create {channel_id,path,label,idempotency_key}` creates a durable opaque RemoteReference with id, channel_id, owner_id, absolute path, display label, restricted=true, source_channels and verified=false. It neither fetches bytes nor claims that the path exists. `reference.get {reference_id}` returns authorized metadata; snapshot includes authorized references. `message.post` and `run.project` accept `references:[id]`, preserving same-channel provenance and restricted classification. Worker creation additionally requires an admitted private-provider run and a path lexically inside its signed remote_root. Actual filesystem use still goes through the separate same-UID helper, including canonical-path and confinement checks. A reference cannot authorize execution or another user's files, and public workers cannot read this metadata.


## Rootless setup checklist

1. Obtain a verified Linux `biorouter-crew` executable for the node architecture. Each Unix account installs its own copy in a directory it owns, typically `~/.local/bin`. Crew does not install system packages, change HOME permissions, create accounts or require sudo.
2. In the desktop connection dialog, expand **Hosting a new workspace?** and choose **Prepare my hosting identity** (`POST /crew/devices/prepare`, proven human only). This persists the private signer locally and returns an opaque preparation ID and public key; recovering preparation after restart returns the same unused identity. Save the eventual real connection with this preparation ID to reuse the exact signer. Keep its private signer on the desktop. As the intended host account, create private home state and launch with that **public** key:

   ```sh
   umask 077
   mkdir -p "$HOME/.local/share/biorouter-crew"
   "$HOME/.local/bin/biorouter-crew" start      --state-dir "$HOME/.local/share/biorouter-crew/lab"      --bootstrap-key DEVICE_PUBLIC_KEY_HEX
   "$HOME/.local/bin/biorouter-crew" status      --state-dir "$HOME/.local/share/biorouter-crew/lab"
   ```

   Start returns a starting PID; status must succeed before sharing connection details. The state parent must exist and the persistent filesystem must be qualified local storage; NFS/SMB remain explicit denials. The ordinary account must resolve through the node's account database.
3. Copy the returned socket, workspace ID, host UID and workspace public key/fingerprint through a trusted human channel into the desktop connection. Strict OpenSSH host verification remains required. Enroll the host device using the signed bootstrap flow. The host creates UID-and-device-key-bound invitations for each colleague, who authenticates through their own SSH account and accepts using their own desktop signer.
4. A root-owned, non-group/world-writable regular `/etc/machine-id` (or standard dbus fallback) supplies the node identity. Crew publishes only a domain-separated SHA256 digest, never the machine-id itself. `hello` signs that digest along with the workspace identity and client challenge. The desktop merges verified aliases/workspaces on the same node under the most restrictive canonical connection policy; association across distinct nodes remains explicit.
5. The workspace persists its canonical writer-node digest. A moved HOME/disk or changed machine identity is refused rather than starting an automatic replacement writer. Recovery to another node is an explicit future administrative application workflow, not a stale-heartbeat takeover.
6. Stop with the same `--state-dir`; the command verifies socket owner, workspace and exact process via pidfd, reports whether termination was observed, and preserves its stable runtime descriptor. The next exclusive writer validates and reclaims the stale socket; stop never races to unlink a replacement writer’s socket. Persistent journal/blobs remain intact for the next start. Workspace keys must be explicitly repinned through trusted recovery if changed.

Every public-worker operation now rechecks whether selected channels acquired restricted messages after admission. That transition denies the next dispatch/read/publication rather than silently supplying a partial context. Worker contribution labels derive from the admitted run's personal mode, not a missing or forged per-tool mode field; remote-derived output remains restricted.

History/search `after` and `before` are optional exclusive bounds containing the random UUID of a currently visible message in the requested channel. The wire `Message.sequence` equals that message UUID; it is not a number or an offset. Numeric anchors are rejected explicitly. Missing, wrong-channel and no-longer-visible string anchors return the same `stale_cursor` refusal; refresh authorized history instead of guessing another position. Absent or null bounds select an open end. `latest:true` selects the newest authorized page within those bounds; each page rechecks current membership and provenance.

The returned `cursor` is the last visible message UUID, or the validated `after` token when the page is empty, or `null` if neither exists. Message post/projection acknowledgments, cached replies, history, search and context manifests all use this projection. Snapshot/read acknowledgments expose the latest currently visible message at or before the internal read watermark, or `null` if none remains visible. Revoking access to a source channel cannot expose that source's former read anchor. Internal journal ordering and durable read watermarks stay numeric and never become public room cursors; existing journals retain their format and message UUIDs across restart.

The workspace policy epoch is a separate workspace-wide authority version. Membership and policy changes invalidate grants globally; it is not a room activity or message counter.


### Capacity and preservation limits

The 16 MiB limit is the serialized logical state, including principals, memberships, runs, all retained messages and cached idempotency responses. Message bodies generally occur twice (history plus the cached response), before metadata and JSON escaping. Consequently, even ignoring all other state, 1 KiB unescaped bodies permit fewer than 8,192 messages and 8 KiB bodies fewer than 1,024. These are upper bounds, not measured capacity: escaping, provenance, attachments and other records lower them. The 100,000-message count ceiling does not override this byte limit. Fifty users posting once per minute for thirty minutes already create 1,500 messages; payload size therefore matters. No 50-user/30-minute acceptance claim is supported without Luna's measured workload results.

`quota_exceeded` leaves existing reads available and refuses new mutations. Channel archival does not free retained history or cached responses. There is no application export, retention editor, audit deletion or online compaction command. For manual preservation, the host must stop the broker, confirm `stopped:true`, prevent a concurrent restart, and copy the **complete** private state directory (journal, lock, blobs, runtime metadata and any quarantine files) to equally private durable storage. Verify the copy using ordinary filesystem checksums before any administrative action. Never truncate or selectively edit the journal or remove cached entries. Continuing writes requires a separately initialized workspace with a new private state directory, new workspace identity and fresh membership invitations; the old store remains the readable historical workspace. This is a preservation/new-workspace procedure, not an in-place retention feature or transparent migration.

`message.post` requires a string `body` of at most 65,536 bytes. It may be empty only when the human message has at least one authorized completed attachment or authorized reference. Worker projections still require a nonempty body.

## Connection admission and job receipt durability

The broker admits at most 256 simultaneous Unix-socket clients and eight per kernel-derived UID. An ordinary cluster account cannot occupy the entire client budget. Slots are released when a client handler exits, including malformed input and unwinding. Idle clients retain the existing five-minute read timeout. These limits bound resource use; they do not claim protection against all host-level denial of service. The 50-user acceptance workload must remain below the documented connection budget and record refusals explicitly.

Before any remote helper starts, its exclusive invocation receipt is written and fsynced, then its directory and any newly created state hierarchy through HOME are fsynced. A failed persistence step prevents execution and leaves an uncertain receipt that cannot be automatically replayed. Completion replaces the receipt atomically and syncs its directory.

Local worker admission is repeated after waiting for the SSH transport and before delivering its response. A revoked, replaced, or policy-stale run cannot use time spent in that queue to retain an earlier Public grant. If policy changes after dispatch, rejection of the returned result does not undo remote effects; the caller must inspect the submitted operation before requesting another grant.
