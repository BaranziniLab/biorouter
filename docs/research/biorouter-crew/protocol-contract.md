# Crew broker protocol v1

> **What this is.** The wire contract of the BioRouter Crew broker (`crates/biorouter-crew`): transport and identity, `hello`, enrollment and joining a workspace, the collaboration methods and what they project, names, attachments, owned-agent grants, durability and the rootless setup checklist.
> **Status:** Current. Protocol version 1; the naming additions leave `PROTOCOL_VERSION` unchanged. The names, renames, projections, `hello` v2 and joining by invitation and device code were added on 2026-09-24 from the [naming design](naming-design.md) (slices S1a, S2a, S2b and S3a). They describe source on branch `codex/biorouter-crew`, not measured acceptance, and joining by invitation is compiled only into a broker built with the `join-by-name` feature, which no release enables yet.
> **Audience:** Implementers of the broker, the daemon's Crew core and the CLI, and reviewers of Crew identity and authorization.

Identifiers: `D1`–`D17` are the naming design's decisions, `SR`/`FR` numbers are findings of its 2026-09-23 security and feasibility reviews, `FU` numbers are its deliberate follow-ups, and `M1`–`M5` are the attackers in its [security analysis](naming-design.md#security-analysis).

Implementation: `crates/biorouter-crew`. The production broker and SSH bridge require Linux; unsupported kernel facilities and unqualified network storage fail explicitly. Product code has been formatted; test/build evidence belongs to the independent Luna lane, not this contract.

## Transport and identity

Run an ordinary-user broker with `biorouter-crew serve --state-dir "$HOME/crew-workspace" --bootstrap-key HEX_ED25519_PUBLIC_KEY`. The state directory's parent must exist. State is owner-only, the initial mode is Private, and the supplied bootstrap key must be the intended host desktop device key. `start` launches the same executable and returns a starting PID; `status` verifies a live matching workspace; `stop` verifies the matching socket peer, workspace and pidfd before sending SIGTERM. Stop waits up to five seconds on the verified pidfd and reports `stopped`; a false value means signal delivered but termination not yet confirmed. No sudo or account/group creation is used.

`serve` and `start` take `--name NAME`, the workspace's name: 1–40 lowercase ASCII letters, digits and hyphens, starting and ending with a letter or digit, and not UUID-shaped. With `--name`, `--state-dir` defaults to `$HOME/.local/share/biorouter-crew/<name>` (parents created 0700). A new workspace takes the name; an existing unnamed one commits it once; a workspace stored under another name is refused (`name_mismatch`). Before it spawns the server, `start` checks that name against the host account's running sibling brokers on the node (at most 32, one second each) and refuses a name one of them uses, so names are unique per host account on a best-effort basis. `start` then waits up to 15 seconds for `hello` and returns `{started_pid, state: "running", status_command, workspace_id, name, invitation}`, where `invitation` is the workspace's [invitation line](#the-invitation) without `ssh_host` (or `null` with `invitation_error`). On timeout it returns the older `{started_pid, state: "starting", status_command}`; if the child exits, it fails `start_failed` with the last line of `broker.log`.

The broker prints `{pid,socket,workspace_id,host_uid,protocol}`. Share the socket path, workspace ID and independently verified workspace-key fingerprint with enrolled colleagues; the [invitation line](#the-invitation) carries exactly these with the host's UID. The runtime basename is selected randomly once, journaled, and reused on the same node across restarts and temporary-directory loss. Existing directory/socket ownership, types and modes are checked; an unexpected occupant or live listener causes an explicit refusal. The directory is node-local and 0711; its socket is 0666, while every request remains authenticated by kernel peer UID and device signature or a limited worker grant. All persistent files and directories are 0600/0700. The hosting UID is trusted: file modes do not isolate hostile code executing under that same UID.

The fixed remote command is `biorouter-crew bridge --stdio --socket PATH --owner-uid UID --workspace-id UUID`. The bridge validates socket/directory ownership, broker SO_PEERCRED and workspace ID before forwarding. The desktop additionally pins and verifies the workspace key via `hello`, before sending enrollment or any protected request. Native OpenSSH verifies remote host identity independently. The broker never accepts a caller-provided username or UID as authority.

Each frame is a newline-terminated JSON object, maximum 1 MiB:

```json
{"version":1,"id":"client-request-id","method":"messages.history","params":{"channel_id":"uuid"},"auth":{"device_id":"sha256-hex","nonce":"challenge","signature":"ed25519-signature-hex"}}
```

Responses are `{id,result}` or `{id,error:{code,message}}`. Unknown operations return `unsupported`; authorization, privacy and expiry failures are explicit errors, never empty-history success. Parameter strings are bounded, IDs opaque, binary file content hexadecimal. At most 256 live connections, with at most eight per kernel UID, are admitted. Each has bounded framing, a write timeout and a five-minute idle timeout. Reconnect performs fresh identity proof and challenge signing.

`hello {challenge_nonce}` returns protocol, workspace_id, host_uid, mode, institution_id, policy_epoch, name, workspace_public_key (raw Ed25519 public key hex), workspace_key_fingerprint (SHA256 of raw public key), node_id (stable machine identity digest), echoed challenge_nonce, signature, signature_v2, capabilities and unsupported facilities. Supply a fresh unpredictable nonce. Changed keys or workspace/host identities require explicit trusted recovery; do not silently repin.

- **v1** `signature` is over the UTF-8 JSON serialization of `[workspace_id,host_uid,challenge_nonce,workspace_public_key,node_id]` (`biorouter_crew::hello_v1_payload`).
- **v2** `signature_v2` is over `[workspace_id,host_uid,challenge_nonce,workspace_public_key,node_id,mode,institution_id,policy_epoch,name,capabilities]`, with `null` for an absent institution or name and `capabilities` in the order `hello` returns them (`biorouter_crew::HelloV2::signing_payload`). Ten elements, so a v2 payload can never equal a v1 payload signed by the same key.
- **Verification.** The daemon requires at least one signature and verifies every one present; a missing or malformed field of a present v2 signature fails the connection, so a relay cannot alter a field that a present v2 signs. Only a verified v2 authenticates the name, mode, institution and policy epoch, and without one the daemon withholds all four.
- **Capabilities without a verified v2.** The daemon keeps them as unauthenticated hints and acts on them: `CrewManager::join_status` sends `enrollment.pending` only when `join_by_name_v1` is present. So a relay on the bridge path that strips `signature_v2` can add or remove capabilities. This fails closed, because a capability only decides which requests the daemon offers, and the broker still authorizes each one. An added capability can only make the daemon send a request the old broker does not have, which it refuses (a join status then reads `unsupported`). A removed one drops the daemon back to the legacy path, such as token enrollment below. The naming design's [wire and version negotiation](naming-design.md#wire-and-version-negotiation) table records this as an accepted case.
- **Capabilities** are `human_chat`, `signed_devices`, `resumable_blobs`, `scoped_runs`, `human_names_v1` (the S1a projections and the broker's `expected_username` check), `unique_names_v1` (S2a unique names and renames) and, only from a broker built with the `join-by-name` feature, `join_by_name_v1`. They describe the broker, not the workspace's identity: the daemon keeps them in memory per connection and never stores them in the saved connection (D12).
- **What a capability may gate, and what it must not.** A capability decides which new methods and screens a client offers; the daemon sends `enrollment.pending` only when `join_by_name_v1` is present. It never decides whether a client sends a defense. A client sends `expected_username` with every [person-targeted mutation](#person-targeted-mutations) whose target the person chose by `@username`, whatever `hello` announced, and both clients do: neither the CLI nor the desktop reads `human_names_v1`. Gating the parameter on that capability would give M2 ([below](#what-a-response-proves)) a downgrade that does not fail closed: strip `signature_v2`, drop `human_names_v1`, and a new broker never receives the field its `target_mismatch` check needs. An older broker reads parameters by name and ignores the field, so sending it there costs nothing, and protects nothing either.

`auth.challenge {device_id}` returns `{nonce,workspace_id,uid,expires_at}`. Device ID is SHA256 of the raw 32-byte device public key. Sign UTF-8 JSON serialization of `[workspace_id,uid,nonce,method,params]`, recursively ordering object keys lexically. The crate's `signing_payload` defines these bytes. Nonces are single-use, bound to the live socket and UID, and expire after 60 seconds. The human key stays in the local trusted device signer; model tools and remote workers never receive it.

`auth.bootstrap {public_key}` is a signed request admitted only once, for host UID and the key pinned at broker creation.

**Legacy enrollment by token.** `enrollment.invite {uid,public_key,idempotency_key}` is host-device signed and creates a one-hour one-use invitation pinned to both UID and intended device key. `auth.enroll {public_key,invitation}` requires proof by that same intended key and kernel UID. Both enrollment operations return `{principal,device_id,workspace}`. Additional devices require another manager-approved key-bound invitation with `existing_principal_id` matching the active principal for that UID. A changed account name is refused until the old principal is offboarded. Acceptance rechecks the exact intended principal. Offboarding and then enrolling creates a fresh UUID without prior memberships. The kernel cannot distinguish a recycled UID with an unchanged username; the manager must explicitly choose add-device versus offboard/new-account intent. `enrollment.revoke {principal_id,idempotency_key}` disables the principal, devices and relevant pending enrollment, and invalidates existing grants. Enrollment itself does not grant team/channel membership or SSH login access.

Naming decision D8 replaces this token path as the designed primary path with [joining by invitation and device code](#joining-by-invitation-and-device-code-s3a). The token path stays for hosts and joiners on older versions and for any broker without `join_by_name_v1`, until two releases after that join ships enabled. Both legacy binds now refuse a key that is already a device (`device_conflict`) and a second active principal with the same canonical username (`identity_conflict`, D3); a successful `auth.enroll` removes any pending join for its UID. `enrollment.revoke` also accepts `expected_username` ([below](#person-targeted-mutations)) and purges pending joins for the revoked UID and for any username that collides with the revoked one.

### What a response proves

Every human request is signed by the person's device key over the method and its parameters, a worker request carries its run credential, and `hello` is signed by the workspace key. **No broker response other than `hello` is authenticated.** A process that sits on the bridge path, for example one running as the caller's own Unix account on the server (attacker M2), can rewrite a snapshot, a join status or any other result without detection (SR3). The contract is built so that such a rewrite can mislead a screen but cannot move authority:

- person-targeted mutations carry `expected_username` inside the signed request, and the broker refuses `target_mismatch` when the target's username differs;
- `auth.join` binds a key only when its device code, computed on the joiner's own computer, equals the code the host approved, and no response ever carries a code.

Authenticated responses (an ephemeral key signed by the workspace key at `hello` and a session MAC over each result) are follow-up FU7, which needs its own review.

## Human collaboration

Every mutation below requires `idempotency_key` in params, signed together with the operation. Keys are scoped to human principal or worker run. Repeating identical bytes returns the same authorized operation result; reusing a key for different bytes fails `conflict`. Cached results are rechecked and projected through current visibility rules before return. Read operations do not require keys.

| Method | Parameters besides mutation key | Result |
|---|---|---|
| `workspace.snapshot` | none | `{workspace,protected_channel_ids,actor,principals,teams,channels,invitations,runs}` filtered to actor-visible objects |
| `channel.read` | channel_id, sequence (opaque visible message token) | Durable monotonic read position; snapshot includes authorized `read_positions` and `unread` maps |
| `profile.update` | nickname, optional avatar | Principal; the display name is validated (below), and `nickname: null` resets it to the username; avatar is at most 12 printable Unicode characters, rendered only as text |
| `profile.suggest` (read) | none | `{full_name}`: the actor's own account full name on the server (the first GECOS field) when it passes display-name validation, else `null`; one account lookup, for the actor only, never on the snapshot path |
| `team.create` | name | `{team,channel}` with automatic general channel; the name must be valid and unique in the workspace |
| `team.rename` | team_id, name | Team; the team's creator, while still a member |
| `channel.create` | team_id, name, classification | Channel; the name is canonicalized to a lowercase slug and must be unique in the team, archived channels included; `general` is reserved; classification `public_safe` or `restricted`, default restricted |
| `channel.rename` | channel_id, name | Channel; the current owner; refused on an archived channel |
| `workspace.rename` | name | Workspace; host only, refused to worker grants, a no-op when unchanged |
| `invitation.create` | kind (`team`/`channel`), target_id, principal_id, optional expected_username | Invitation; target must accept |
| `invitation.accept` | invitation_id | `{accepted:true}` |
| `channel.archive` | channel_id | Channel; history retained, future writes denied |
| `channel.transfer` | channel_id, successor_id, optional expected_username | Channel with pending_owner; successor must already belong and be active |
| `transfer.accept` | channel_id | Channel; owner changes, immutable created_by retained |
| `membership.revoke` | channel_id, principal_id, optional expected_username | Channel; current owner required, cannot remove self |
| `message.post` | channel_id, body, optional attachments, personal_mode | Message; mode missing/unknown conservatively marks contribution restricted |
| `messages.history` | channel_id, optional after, before, limit, latest | `{messages,cursor}`; limit defaults 100, maximum 200 |
| `messages.search` | channel_id, query, optional after, before, limit, latest | same, authorized literal case-insensitive text search |
| `policy.set` | mode (`private`/`public`) | Workspace; host-device only, increments policy epoch |

Persisted data types live in `src/lib.rs`; wire projections replace internal message sequence counters with opaque tokens. Principal includes id, verified uid, canonical username, nickname, textual avatar, active. Channel includes id, team_id, name, created_by, owner_id, members, archived, classification, pending_owner. Message includes id, sequence, channel_id, actor_id, optional run_id, body, created_at (Unix seconds), restricted, source_channels, attachments, optional status. A team's general room adds accepted team members automatically. Restricted channels require their current owner's invitation. Transfers remove the previous owner's implicit membership (a separate successor invitation is needed to rejoin) and invalidate old-owner invitations and all worker grants through the global policy epoch. Joining a channel never grants access to another channel's derived content.

### Names

Names are display and lookup keys, never authority or primary keys: every ACL, membership, owner, author and invitation field stays an ID, and a rename keeps the object's ID, history, invitations and grants. No rename bumps the policy epoch. The rules live in `crates/biorouter-crew/src/names.rs`; the naming design's [workspace, team and channel names](naming-design.md#workspace-team-and-channel-names) explains them.

- **Workspace:** the ASCII slug set by `--name` (above) or `workspace.rename`.
- **Team:** a cleaned display name of 1–64 characters, with a derived `handle`, unique in the workspace.
- **Channel:** a lowercase slug, unique in its team including archived channels; `general` is reserved for the team's first channel.
- **Refused in every name:** a name that parses as a UUID or 64 hex characters, `@` and `#` (reserved for selectors), control and invisible characters, and, for teams and channels, mixed scripts beyond UTS #39 Highly Restrictive.
- **Uniqueness** compares the name key (NFKC, lowercase, separators folded, invisible characters removed) and the UTS #39 confusable skeleton (`names_collide`), so case, spacing, look-alike letters and invisible characters cannot make a second name. Renaming only the case or spacing of an object's own name is allowed.
- **Legacy journals replay unchanged** (D6). Validation applies only to creates and renames; an old name that breaks the rules is flagged in projections, never rewritten.

A collision is refused `name_taken` with one fixed sentence whether or not the caller can see the object holding the name, carrying no ID, creator or colliding spelling (D5). Each `name_taken` refusal counts toward an in-memory, per-actor limit of ten in ten minutes; while it is exceeded, team and channel create and rename and `workspace.rename` answer `rate_limited` ("Too many name attempts. Try again later.") before validating the name. An invalid name is `name_invalid`, whose message never echoes the input.

### One active principal per username

No two active principals may share a canonical username, compared after case folding and the confusable skeleton (D3). Legacy `auth.enroll`, both forms of `enrollment.invite` and `auth.join` refuse a second one with `identity_conflict`. A legacy journal that already holds such a pair is left as it is: the host's snapshot marks the principal whose UID no longer maps to its username with `account_stale: true`, and the host removes it.

### Person-targeted mutations

`invitation.create`, `membership.revoke`, `channel.transfer` and `enrollment.revoke` accept an optional `expected_username`: the `@username` the person confirmed when naming the target. It is part of the signed request, so it cannot be altered on the bridge path, and the broker refuses `target_mismatch` when the target principal's username differs. Clients send it whenever the person named the target by username, whatever capabilities `hello` announced, because a capability without a verified v2 can be stripped on the bridge path ([transport and identity](#transport-and-identity)).

### Projections for names

All of these are display-only, computed at projection time and never journaled:

- **Snapshot:** `workspace.name` and `workspace.host_principal_id`; each principal's `display_name` (the sanitized nickname, else the username); `former_principals` (`{id, username, display_name, avatar, active: false}` for inactive principals referenced by the actor's visible teams, channels and invitations); on visible invitations `target_name`, `team_name` (channel invitations), `inviter {username, display_name}` (null when the inviter is missing) and `expired`, with expired invitations omitted for the invitee and marked for the inviter; on teams and channels `handle` (the name key, for channels too), a sanitized `display_name`, and `name_conflict` and `name_invalid` (booleans, computed only among objects the viewer can see); and `actor.devices` (`[{fingerprint, added_at, added_via}]`, the fingerprint being the grouped first 16 hex digits of the device ID and `added_via` one of `bootstrap`, `token` or `invitation_code`, both null for legacy devices).
- **Host's snapshot only:** `account_stale: true` where it applies, `name_collision_refusals {principal_id: count}`, and `pending_joins` ([below](#methods)).
- **Message results** (history, search, `message.post`, `run.project` and `context.manifest`): one `people` map `{principal_id: {username, display_name, active}}` per response, and `channel_names {channel_id: name}` only for channels the actor can read, checked at runtime. `message.post` and `run.project` return the message itself, so both maps are added to that object.

## Joining by invitation and device code (S3a)

The host invites an account by its username on the server; the joiner pastes the host's invitation, which pins the workspace; the joiner's own computer computes a device code from its device key and the pinned workspace key; the host types that code to approve; and `auth.join` binds a key only when its code equals the approved one (D8). The broker never sends a code, and nothing it or a process in the bridge path returns can change the code the joiner's screen shows.

This is compiled only into a broker built with the `join-by-name` cargo feature, which then advertises `join_by_name_v1`. **No release build enables the feature** until a mandatory independent adversarial identity and authorization review, the hostile-bridge harness, the previous-release-binary downgrade test and a live three-account run all pass (D17). Without it, `enrollment.pending` and `auth.join` fall through to authentication and fail closed, the new form of `enrollment.invite` is refused, and the [legacy token path](#transport-and-identity) is the only enrollment.

### The invitation

```text
Join lab on Crew.
In Biorouter, open Crew, choose Join a workspace, and paste this whole message.
brcrew1:<base64url(JSON), unpadded>
```

```text
JSON = {
  "v": 1,
  "workspace_id": "<uuid>",  "workspace_public_key": "<64 hex>",
  "socket_path": "/tmp/<runtime dir>/<socket>",  "owner_uid": 1000,
  "workspace_name": "lab" | null,
  "host_username": "alice",  "host_display_name": "Alice Chen" | null,
  "mode": "private" | "public",  "institution_id": "ucsf" | null,
  "ssh_host": "hpc.ucsf.edu" | null,  "ssh_port": 22 | null,  "proxy_jump": "gateway.ucsf.edu" | null,
  "invitee_username": "bob" | null
}
```

- **Provenance.** The host's daemon builds the message (`GET /crew/connections/{id}/invitation`) from the host's own verified connection (the four pinned fields), the last verified snapshot (name, mode, institution, host username) and `ssh -G` for the host's SSH target (host name, port and jump host, never a local alias). `biorouter-crew start --name` prints the same line without `ssh_host`. The workspace key therefore still reaches the joiner from the host over a human channel; there is no trust-on-first-use change.
- **Parsing.** Only the daemon parses it (`biorouter_crew::invitation::parse`): the first `brcrew1:` token anywhere in the pasted text, else the legacy JSON `biorouter-crew status` prints. At most 64 KiB of pasted text is scanned and 4 KiB of decoded JSON accepted. An unknown `v`, an unknown field, an invalid pinned field (a non-canonical workspace UUID, a key that is not a valid Ed25519 point, a socket outside `/tmp/<dir>/<socket>` or longer than 107 bytes, UID 0) or oversize input is refused with an `invitation_*` code.
- **Labels are not authority.** `workspace_name`, `host_display_name` and the SSH hints are display metadata and defaults. `hello` must verify against the pinned workspace key before anything is trusted, and the broker still enforces the institution match for protected context.
- **Privacy (D9).** `mode` and `institution_id` prefill the joiner's connection, and the joiner confirms them before anything is saved, so a Private save always has an institution.
- **Integrity, not secrecy.** An invitation reveals nothing a server user cannot learn from `hello`, plus the host's chosen labels. Forwarding it to the wrong person gains them nothing without an account the host invited and the host's approval of their device code.

### The device code

```text
device_code(workspace_id, W, K) =
  Crockford-base32( SHA-256( "biorouter-crew-device-code-v1\0" ‖ workspace_id ‖ "\0" ‖ W ‖ K )[0..10] )
  → 16 characters, displayed 7QK2-M9XA-3JTP-WZ4D
```

`W` is the 32-byte workspace public key the joiner pinned and `K` the joiner's 32-byte device public key (`biorouter_crew::invitation::device_code`). Input is normalized by removing spaces, dashes and invisible characters, upper-casing, and reading `I` and `L` as `1` and `O` as `0`; a `U`, any other character or a length other than 16 is refused (`device_code_invalid`). The broker compares codes in constant time.

- **Computed at both ends, shown only by the joiner.** The joiner's daemon computes it from its saved key and pinned `W`; the broker computes it from the claimed key and its own `W` when checking `auth.join`. No broker response contains a code.
- **Why 80 bits.** To be bound, a relaying attacker must present a key whose code equals the one the joiner's screen shows: a second preimage on 80 bits, infeasible within an invitation's 24-hour lifetime.
- **Why `W` is inside.** An invitation tampered in transit to carry another workspace key yields a code the real broker never accepts, so it cannot admit anyone. This is defense in depth only; the two-sided code that discovery would need is deferred with discovery itself (D10, slice S4).

### Methods

| Method | Caller and gate | Params | Result | Journal |
|---|---|---|---|---|
| `enrollment.invite` (new form) | Signed human; host (`manager`), checked before any account lookup | `{username, add_device?}` | `{username, full_name, add_device, expires_at}` | Mutation |
| `enrollment.approve` | Signed; host | `{username, code, replace?}` | `{approved: true, username}` | Mutation; approving the same code again is idempotent, and a different code needs `replace: true` (else `already_approved`) |
| `enrollment.cancel` | Signed; host | `{username}` | `{cancelled: true}` | Mutation |
| `enrollment.pending` | **Pre-authentication**, like `hello`; the kernel UID only; no account lookup | `{}` | `{invited: false}`, or `{invited: true, join_id, workspace_name, inviter: {username, display_name}, add_device, approved, expires_at, expired, last_refusal?: "code_mismatch"}` | Read |
| `auth.join` | **Pre-authentication**, like `auth.enroll`; signed by the claimed key over an `auth.challenge` nonce | `{public_key, join_id}` | `{principal: {username, display_name}, device_id, workspace}` | Commits directly: operation `auth.join`, actor `uid:<n>` |
| Host's snapshot | — | — | `pending_joins: [{username, full_name, add_device, approved, created_at, expires_at, expired, mismatched_attempts}]`, never a code, key, UID or join ID | — |

`expired` is computed on the broker's clock, so skew between a desktop and the server cannot mislead either side. The `enrollment.invite` result omits the join ID because mutation results are cached for retries, and a cached result must never carry one.

**A typed name is canonicalized before it is used.** One leading `@` is stripped. The name must be 1–256 bytes, with no NUL, control, whitespace, `/`, `:`, invisible or bidirectional-control characters, and not all digits (`name_invalid`). The broker looks the account up by name and then by that account's UID, and both must give the same name: an alias is refused `identity_ambiguous`, naming the canonical account, and so is a spelling that differs only in case. An account whose own name breaks the same rules is refused `identity_unavailable`, and an unknown name `unknown_account`. Names are looked up on submit only, one point lookup at a time, never listing accounts, and cached for 30 seconds because account lookups can block on LDAP under the broker's lock.

**`enrollment.invite` (new form) checks, in order:** the caller is the host; the name canonicalizes; an active principal with this UID and username requires `add_device` (`already_member`), and `add_device` requires one (`invalid_params`); an active principal on this UID under another username is refused `identity_mismatch`; the same canonical username held by an active principal, or invited for a pending join, on another UID is refused `identity_conflict`. The join records the UID, canonical username, the account's full name (a label only), the inviter, and the principal IDs holding that UID or username at invite time (its generation). It expires 24 hours later. Inviting again replaces the UID's join with a new `join_id`. At most 100 joins may be pending (`quota_exceeded`); expired joins are pruned before every mutation and never count.

**`auth.join` checks, in order; any refusal writes nothing:**

1. A pending join exists for the kernel UID (`not_invited`), has not expired (`join_expired`) and carries the given `join_id` (`join_changed`, so the joiner re-reads its status).
2. The account name for that UID still equals the invited username (`account_changed`: guards renames and recycled UIDs).
3. The principals holding that UID or username still equal the recorded generation; for another device, the existing principal is still active with the same UID and username (`account_changed`).
4. The caller proves possession of the key with a single-use, 60-second, socket-bound `auth.challenge` nonce.
5. The key is not already a device (`device_conflict`).
6. An approved code is set and equals `device_code(workspace_id, W, public_key)`. Otherwise the claim is refused `code_mismatch`, counted in the join's `mismatched_attempts` (the host sees "a device with a different code tried to join as @bob" before approving), and reported to the joiner as `last_refusal` while the same approval stands. A claim before any approval is refused the same way, which is why a client claims only after `enrollment.pending` reports `approved: true`.
7. One active principal per username (D3) still holds.

On success the broker creates the principal, with its nickname set to the username (D2), or selects the existing one for another device, inserts the device with `added_via: "invitation_code"`, removes the join and any legacy enrollment for the UID, and commits.

**Lifecycle.** `enrollment.revoke` purges pending joins for the revoked UID and any colliding username. A successful legacy `auth.enroll` removes its UID's pending join. The approved code is journaled with the join so an approval survives a broker restart; mismatched attempts, `last_refusal` and the name cache are in memory only, so a restart drops warnings, never an approval.

**State (D11).** Joins live in a top-level `pending_joins` map keyed by decimal UID and serialized only when non-empty. The first join journals a `Set [pending_joins]` and the last removal a `Remove [pending_joins]`; both replay on older and newer brokers, so there is no upgrade record and no write when a broker opens. An older broker ignores the map. `Workspace.name` is likewise serialized only when set, and `Device.added_at` and `Device.added_via` default when absent.

**The daemon's side.** The daemon parses invitations and serves the join through `POST /crew/connections/from-invitation`, `GET /crew/connections/{id}/invitation`, `GET /crew/connections/{id}/join` and `POST /crew/connections/{id}/join`, each requiring proof of a person. It sends `enrollment.pending` unsigned over the bridge, like `hello`; computes the code locally; and sends `auth.join` only through its own signed join path, never through the generic request route, which refuses both methods.

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

The bridge also refuses (`invalid_scope`) a work folder that is, contains or sits inside a protected folder, so a confined worker can never replace the bridge or wrap it through a shell startup file and become the in-path process of the join's threat model (D15, SR4). The protected folders are `~/.ssh`, `~/.aws`, `~/.config`, `~/.local/state`, `~/.local/share/biorouter-crew`, `~/.local/bin`, `~/bin`, `~/.bashrc.d` and `~/.profile.d`, plus the resolved bridge executable's directory and its ancestors inside `HOME`, under every spelling of `HOME` and of the bridge path. The refusal suggests a folder such as `~/crew-work/<workspace>`. Protecting directories on the remote login `PATH`, or allowing only folders under a Crew-created prefix, is follow-up FU9.

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

   Add `--name lab` to name the workspace; with it, `--state-dir` may be omitted and defaults to `$HOME/.local/share/biorouter-crew/lab`. Start waits up to 15 seconds and returns the invitation line when the broker answers, or a starting PID when it has not answered yet; status must succeed before sharing connection details. The state parent must exist and the persistent filesystem must be qualified local storage; NFS/SMB remain explicit denials. The ordinary account must resolve through the node's account database.
3. Copy the returned socket, workspace ID, host UID and workspace public key/fingerprint through a trusted human channel into the desktop connection. The invitation line `start` returns carries exactly those four pinned fields, so the host can save the connection from it with the prepared identity instead of typing them. Strict OpenSSH host verification remains required. Enroll the host device using the signed bootstrap flow. With a `join_by_name_v1` broker the host then [invites each colleague by username](#joining-by-invitation-and-device-code-s3a) and approves the device code their desktop shows; otherwise the host creates UID-and-device-key-bound token invitations for each colleague, who authenticates through their own SSH account and accepts using their own desktop signer.
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

## Related documentation

- [Naming design](naming-design.md) — the decisions, threat model and tests behind names, `hello` v2 and joining by invitation
- [Native CLI guide](cli-guide.md) — the commands that call these methods, including joining and the legacy token path
- [Implementation plan](implementation-plan.md) — §5 identity and authorization, §6 device authority and §16 naming requirements
- [Implementation status](implementation-status.md) — which parts have evidence, and the S3a release gate
- [SSH hop policy](ssh-hop-policy.md) — host-key trust rules the invitation leaves unchanged
- [Linux portability](linux-portability.md) — the broker's supported kernels, filesystems and packaging
