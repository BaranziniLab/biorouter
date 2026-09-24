# Crew human-readable identity and naming design

> **What this is.** The design for how BioRouter Crew refers to people, workspaces, teams and channels by memorable names instead of machine IDs, how a person is invited and joins a workspace by `@username` without copying machine strings, and the exact code, compatibility, test and review changes needed, cut into shippable slices.
> **Status:** Current. Approved design, 2026-09-23, revised after an adversarial security review and a feasibility review of the first draft. Nothing in it is built yet; [implementation status](implementation-status.md) records progress.
> **Audience:** Implementers and reviewers of the broker (`crates/biorouter-crew`), daemon core and routes, CLI and GUI; the independent reviewer who signs off the join slice; the docs keeper.

The first draft of this design (join by host `@username`, workspace discovery, and an 8-digit code computed by the
broker) was reviewed twice on 2026-09-23. Both reviews rejected the join as designed: the code reached the joiner
through the joiner's own bridge binary, which a process running as the joiner's Unix account can replace, so that
process could get its own key admitted as the joiner. This version keeps what the reviews confirmed (the identity
model, one active principal per username, name keys that are never stored, legacy journals that replay unchanged,
a daemon resolver over the caller's own snapshot) and replaces the join with a host-issued invitation plus a device
code the joiner's own computer computes. Discovery and short numeric codes move to a later slice with their own
review. Every finding of both reviews is answered below; the few that are deferred say why.

## Identifiers used in this document

- **Decisions `D1`–`D17`** are listed in [Decisions](#decisions) and are what the docs keeper records.
- **Slices `S0`, `S1a`, `S1b`, `S2a`, `S2b`, `S3a`, `S4`** are defined in [Slices, order and review gates](#slices-order-and-review-gates).
- **`SR1`–`SR15`** are the findings of the adversarial security review (its F1–F15) and **`FR1`–`FR20`** the findings
  of the feasibility review (its R1–R20), both dated 2026-09-23.
- **Attackers `M1`–`M5`** are defined in [Security analysis](#security-analysis).
- **`FU1`–`FU10`** are deliberate follow-ups ([Deferred follow-ups](#deferred-follow-ups)).
- `plan:n` is a line of [the implementation plan](implementation-plan.md); `I01`–`I24` are the invariants in
  [implementation status](implementation-status.md).
- Paths: `lib.rs`, `broker.rs`, `main.rs`, `remote.rs` are in `crates/biorouter-crew/src/`; `core/` is
  `crates/biorouter/src/crew/`; `routes/` is `crates/biorouter-server/src/routes/`; `cli/` is
  `crates/biorouter-cli/src/commands/crew/`; `ui/` is `ui/desktop/src/components/crew/`. Line numbers refer to
  `76b88555`.

## Decisions

| # | Decision | Answers |
|---|---|---|
| D1 | **People are shown by display name with `@username` next to it.** At every authority decision point both are shown in full, and wherever two display names collide. A UUID, key or numeric UID is never shown by default. | plan §16 |
| D2 | **The display name is the existing `nickname` field, and it defaults to the username.** The account's full name (the first GECOS field) is *offered* as a visible, prefilled suggestion after joining and in Edit profile; it is never applied silently. `nickname: null` resets to the default. | SR8, FR13, FR20 |
| D3 | **At most one active principal per canonical username**, compared after case folding and, from S2b, the confusable skeleton. Enforced wherever a principal is created or re-activated. The principal UUID remains the enrollment generation. | SR9 |
| D4 | **One rule per kind of name.** Workspace: an ASCII slug (`lab`), unique per host account (best effort). Team: a display name (`Analysis Lab`) plus a derived handle (`analysis-lab`), unique per workspace. Channel: a lowercase slug (`#methods`), unique per team including archived channels, with `general` reserved. Names that parse as a UUID or 64 hex characters are refused. The broker enforces all of it on create and rename. | FR8, SR5, SR14 |
| D5 | **The name-existence oracle is accepted, and bounded.** Collision refusals use one wording whether or not the colliding object is visible, are rate limited per actor, and `name_conflict` is computed only among objects the viewer can see. | SR6 |
| D6 | **Legacy journals are never rewritten.** Keys, flags and sanitized names are computed, never stored. Legacy duplicates stay readable, are flagged to their owners, resolve as ambiguous and are fixed by renaming. | FR5 |
| D7 | **Names are resolved in the daemon, from the caller's own snapshot, by one resolver** (`POST /crew/resolve`, connection-independent). Authority-bearing inputs accept only `@username` or an ID for people, never a display name, and never fall back silently on case. UUID-shaped input is always an ID. Person-targeted mutations carry `expected_username`, which the broker checks. | FR8, SR3, SR9, SR14 |
| D8 | **Joining uses a host-issued invitation and a device code computed on the joiner's desktop**, which the host pastes to approve. The broker never sends a code. The enrollment token remains a legacy path until a named release. | SR1, FR1, FR11 |
| D9 | **The invitation is the host's verified descriptor plus the workspace's privacy mode and institution**, one line (`brcrew1:`) inside a human-readable message, parsed only by the daemon. The joiner confirms the workspace's privacy before saving. | FR3.4, FR10, SR7 |
| D10 | **Discovery by host `@username` and 8-digit short codes are deferred to S4**, which requires a two-sided commit-then-reveal code that binds the workspace key, SSH-host-identity cluster merging, a no-re-pin rule, a bounded scan, its own adversarial review and a maintainer decision on the change of trust provenance. | SR2, FR3 |
| D11 | **New broker state serializes only when non-empty** (`skip_serializing_if`), so no upgrade record and no write on open are needed, and older brokers replay new journals. | FR2, SR13 |
| D12 | **Capabilities are not identity.** Broker capabilities live in daemon memory, not the persisted connection. `hello` gains a v2 signature over the workspace name, mode, institution, policy epoch and capabilities; `PROTOCOL_VERSION` stays 1. | FR9, SR7 |
| D13 | **Legal names reach a model only after the person chooses them.** Contexts sent to a public provider use the display name only when the person set or confirmed it, otherwise `@username`. | FR20, SR8 |
| D14 | **Agent labels come from admission, never from a human-signed call on the worker path.** A task conversation is titled after admission. | FR6 |
| D15 | **A remote work folder can never contain the bridge.** `~/.local/bin`, `~/bin`, `~/.bashrc.d`, `~/.profile.d`, and the resolved bridge executable's directory and its ancestors up to `HOME` join the protected list; the GUI suggests `~/crew-work/{workspace}`. An allowlist-only rule is a follow-up because it would break saved folders. | SR4 |
| D16 | **A person can see the devices on their account.** The snapshot projects the actor's devices, and a new device raises a one-time notice on the person's other computers. | SR15 |
| D17 | **Slice order:** S0 → S1a and S1b → S2a (S2b any time after) → S3a, compiled behind the `join-by-name` cargo feature until its adversarial review and the live three-account run pass → S4 later. | FR §9, plan §16.5 |

## Principles

- **Names are for display and lookup; IDs are for authority and keys.** Nothing is re-keyed by name. Every ACL,
  membership, owner, author and invitation field stays a principal, team or channel UUID (plan §5, §16.5).
- **The principal UUID is the enrollment generation.** Re-enrolling after offboarding mints a new UUID with no
  memberships (`broker.rs:1004-1011`); every name-addressed invitation binds the UUID or the UID-and-username it
  resolved to. No other generation concept is added.
- **Names are scoped to one workspace.** `@bob` in two workspaces may be two people (different nodes, different NSS
  realms). A cross-workspace view qualifies names (`@bob · lab`).
- **Visibility bounds disclosure.** Any name projected to a caller is one that caller could already reach through the
  membership-filtered snapshot, except the deliberate, listed cases.
- **One resolver, one set of labels.** The CLI and GUI both call the daemon; the daemon projects display labels so
  the collision rule is implemented once (FR19).
- **Security-relevant display is honest about its source.** Broker responses other than `hello` are not signed
  today (SR3); the design says so where it relies on them, and binds every person-targeted mutation to the
  `@username` the person confirmed.

## People

### The display rule

| Context | Format | Example |
|---|---|---|
| Message header, people list, member rows | **Display name**, then `@username` as muted secondary text in its own element; `@username` alone when the two are equal case-insensitively | **Bob Lee** `@bob` |
| Compact inline text (notes, toasts, invitation text, run owner tags, CLI lines) | `Display name (@username)`, or `@username` when equal | `Bob Lee (@bob) invited you to Analysis Lab` |
| Authority decision points (invite, remove member, offer or accept ownership, offboard, approve an agent request, grant consent) | Always `Display name (@username)`, plus `· former member` when inactive, even if equal | `Remove Bob Lee (@bob) from #methods?` |
| Admitting a joiner (host) | `@username` first, in monospace, then "{full name} (name on the server account)" | `@bob` · Bob Lee (name on the server account) |
| Chips, avatars, @-mention autocomplete | Display name with `@username` in a tooltip; both on a collision | — |
| Former member | Label plus ` · former member`, muted | `Bob Lee (@bob) · former member` |
| ID in no projection | `Unknown member`, with the ID only under Copy ID | — |
| CLI human output and model-facing text | `"Display name" (@username)`, the display name quoted and wrapped in Unicode isolates (U+2068 … U+2069) | `"Bob Lee" (@bob) · 14:02  The analysis is ready.` |

**Collision rule.** Build the directory of one workspace: active principals, the former principals projected below,
and the host. Two entries collide when their display names share a `skeleton_key` (from S2b; `name_key` before it).
Colliding entries render `Display name (@username)` in every context, chips included. The daemon computes this and
projects `labels: {principal_id: {full, short, collides}}` on each observation `state` frame; the GUI and CLI use
those labels and fall back to a local computation only when a daemon predates them (FR19).

### Where display names come from

- **Wire field.** `Principal.nickname`, unchanged for compatibility. Projections add a computed, read-only
  `display_name` (the sanitized nickname, else the username).
- **Default.** A new principal's nickname is its username. The broker does not read GECOS when binding a device.
- **The name on the server account.** A new read method, `profile.suggest` (signed, actor only), returns the actor's
  own first GECOS field after the display-name validation below, or nothing. The GUI calls it when Edit profile opens
  and once after joining, where it offers "Use “Bob Lee” as your name in lab?". One NSS lookup per call, for the
  actor only, never on the snapshot path (FR14).
- **Setting it.** `profile.update {nickname, avatar}` as today; `nickname: null` resets to the username (FR13).
  `biorouter crew profile set NAME` keeps working and gains `--name` and `--reset`.

### Display-name validation

Applied by `profile.update` and to `profile.suggest` output. Cleaning: NFC, trim, collapse runs of `White_Space` to
U+0020. After cleaning the name must:

- be 1–64 Unicode scalar values and at most 120 UTF-8 bytes;
- contain at least one visible base character of category L or N that is not default-ignorable;
- contain no character of category Cc, Cf, Co, Cs, Cn, Zl or Zp, and no `Default_Ignorable_Code_Point` (variation
  selectors U+FE00–U+FE0F and U+E0100–U+E01EF, Hangul fillers U+115F, U+1160, U+3164, U+FFA0, and the rest of the
  property, embedded as a generated table so no new dependency is needed in S1a) (SR5);
- contain no `@` or `#` after NFKC, and, from S2b, none after the confusable skeleton either, so fullwidth `＠` and
  small `﹫` are refused (SR5);
- not equal, by `name_key` (S1a) or `skeleton_key` (S2b), the username of any **other** principal, active or former,
  or the host. A person's own username is allowed.

Mixed scripts are allowed: real names mix scripts (`李明 Li Ming`), display names confer no authority, and
`@username` appears at every authority point. The **projection sanitizer** strips rejected characters from legacy
nicknames and falls back to the username when the result is empty, too long, or looks like another principal's
username. The skeleton checks move to S2b because the `unicode-security` crate arrives there (FR12).

### Username uniqueness among active principals

- **The rule (D3).** No two active principals may share a canonical username, compared after case folding (S1a) and
  the confusable skeleton (S2b). Enforced at legacy `auth.enroll`, both forms of `enrollment.invite`, and S3a's
  `auth.join`. The refusal: `identity_conflict: another active member is @bob; remove the old @bob first`.
- **Why.** Today only the UID is checked (`broker.rs:1003`). If `bob` (UID 1001) is deleted and a new `bob` (UID 1050)
  is created, enrolling the new account would make `@bob` ambiguous.
- **Legacy journals may already hold such a pair.** Nothing is offboarded automatically. The **host's** snapshot marks
  the principal whose UID no longer maps to its username with `account_stale: true` (one NSS lookup per conflicting
  pair, host snapshot only; FR14); the resolver reports `@bob` as ambiguous; the host sees "Remove the old @bob
  (account no longer valid)".

### Where `@username` must appear

Every row of a person picker; remove member; ownership offer and accept; offboard (the typed-username confirmation
stays, now case-sensitive); the host's admit dialog; agent ownership labels; approval prompts; anything a model
writes about a person.

### Former members and authors

The broker is the only party that knows former principals, so it projects them. All of it is display-only.

- **`former_principals`** (snapshot): `{id, username, display_name, avatar, active: false}` for every inactive
  principal referenced by the actor's visible objects: the `members`, `created_by`, `owner_id` and `pending_owner` of
  visible teams and channels, and the `principal_id` and `inviter_id` of visible invitations. Bounded by team and
  channel sizes.
- **`people`** (message results): one map per response, `{principal_id: {username, display_name, active}}`, beside
  `messages` in history, search, post, `run.project` and `context.manifest` results, instead of a per-message author
  object that would repeat up to 200 times against the 1 MiB frame limit (FR15). Worker reads get the same map.
- **Channel names on messages** (`channel_names: {channel_id: name}` in the same results): included only for channels
  where `channel(s, actor, id, false)` succeeds, checked at runtime in release builds, never by a `debug_assert!`
  (FR15).
- **Host detection** uses `workspace.host_principal_id`, injected at projection time and never stored in
  `Workspace` (it would be journaled otherwise).
- **Devices (D16).** `actor.devices: [{fingerprint, added_at, added_via}]`, where `fingerprint` is the grouped first
  16 hex of the device ID, and `added_via` is `bootstrap`, `token` or `invitation_code`. `Device` gains `added_at`
  and `added_via` with `#[serde(default)]`.

## Workspace, team and channel names

### The three kinds

| Object | Stored | Shown | Addressed in the CLI | Unique within | Who renames |
|---|---|---|---|---|---|
| Workspace | `Workspace.name: Option<String>`, ASCII slug | `lab · hosted by Alice Chen (@alice)` | the saved connection name after joining | The host account's workspaces on that node (best effort) | Host: `biorouter-crew start --name` and `workspace.rename` |
| Team | `Team.name`, the cleaned display name | `Analysis Lab` | `analysis-lab` or `"Analysis Lab"` | The workspace | Team creator (`team.rename`) |
| Channel | `Channel.name`, a slug | `#methods` | `methods`, `analysis-lab/methods` (a leading `#` is optional) | The team, archived channels included | Current owner (`channel.rename`) |

A workspace name travels in remote SSH commands, which must pass `safe_atom` (`core/mod.rs:276-282`), so it is
ASCII. Teams are named like organizations, so they keep a display name with a derived handle. Channels follow the
`#kebab-case` convention; the slug is its own handle.

### Normalization and keys

Added to `lib.rs` so the broker and the daemon share one definition (the core already depends on this crate):

```text
clean(s)        = NFC(s) → trim → collapse White_Space runs to ' '
strip_ignorable = remove Default_Ignorable_Code_Point characters
name_key(s)     = strip_ignorable(NFKC(s)) → to_lowercase() → NFKC
                  → map ' ', '-', '_', '.' to '-' → collapse '-' runs → trim '-'
skeleton_key(s) = name_key( UTS #39 skeleton( strip_ignorable(NFKC(s)) ) )     (S2b)
```

- `to_lowercase` is Rust's default mapping, not full case folding, so `ß` and `ss` stay distinct; the risk for lab
  names is negligible and accepted.
- Keys are never persisted. There are at most 100 teams and 1,000 channels per scope (`broker.rs:1665`, `:1707`).
- `"Analysis Lab"`, `"analysis-lab"`, `"ANALYSIS_LAB"`, `"Analysis.Lab"`, fullwidth `"Ａｎａｌｙｓｉｓ Ｌａｂ"` and
  `"Analysis Lab"` followed by U+FE0F all have the key `analysis-lab`, so a variation selector cannot create a
  lookalike (SR5).
- **Unicode data drift (SR13).** The desktop daemon and the node's broker are installed separately and may carry
  different Unicode tables. The broker therefore projects each team's and channel's computed `handle` in the
  snapshot, and the daemon matches against that; when the daemon's own key disagrees with the projected handle, the
  resolver answers "ambiguous" rather than guessing.

### Validation per kind (create and rename only)

**Team display name.** After `clean()`: 1–64 scalar values and at most 120 bytes; at least one L or N; only
characters of category L, M or N, the space, and `- _ . ' & ( ) +`; every code point with UTS #39
`Identifier_Status=Allowed` (S2b; before S2b, the category and default-ignorable rules only); no default-ignorable
characters; no combining mark that does not compose under NFC; no `@`, `#`, `/` or `:` (reserved for selectors); no
emoji or symbols; a non-empty `name_key` that does not parse as a UUID or 64 hex characters; and the restriction-level
rule below.

**Channel name.** The broker canonicalizes first (NFKC, lowercase, whitespace and `.` to `-`, collapse runs, trim),
then requires: every character a lowercase-stable letter, a composing mark, a decimal digit, `-` or `_`; the first
character a letter or digit; 1–80 scalar values and at most 120 bytes; `Identifier_Status=Allowed` (S2b); no
default-ignorable characters; not UUID-shaped; and the restriction-level rule. Canonicalizing instead of refusing
matches Slack, keeps older clients that send `"Data Analysis"` working, and lets the GUI preview the exact slug.

**Workspace name.** 1–40 lowercase ASCII letters, digits and hyphens, starting and ending with a letter or digit
(`^[a-z0-9]{1}(?:[a-z0-9-]{0,38}[a-z0-9])?$`), and not UUID-shaped.

### Confusable characters (S2b)

- **Restriction level.** Team and channel names are at most UTS #39 Highly Restrictive: one script plus Common and
  Inherited, or Latin with Han and Hiragana and Katakana, Latin with Han and Bopomofo, or Latin with Han and Hangul.
  This refuses `Аnalysis` with a Cyrillic А and allows `Lab 分析`.
- **Lookalike refusal.** A create or rename is refused when its `skeleton_key` equals that of another team in the
  workspace, or another channel in the team (`anaIysis` beside `analysis`; `rn` beside `m`). Rare false positives are
  accepted; the message says the name "looks too much like" an existing one.
- **Dependency.** `unicode-security` (with `unicode-script`), pinned exactly as the crate pins its other
  dependencies, after a supply-chain review. `unicode-normalization` and `unicode-properties` are already in
  `Cargo.lock`.

### Uniqueness scope and enforcement

- **Teams:** `team.create {name}` and `team.rename {team_id, name}` refuse a key or skeleton key that matches any
  **other** team in the workspace, whatever the caller's membership. Renaming only the case or spacing of a team's own
  name is allowed.
- **Channels:** `channel.create {team_id, name}` and `channel.rename {channel_id, name}` refuse a match with any other
  channel in the team, archived ones included. `general` is reserved except for the auto-created channel
  (`broker.rs:1681`). The same slug in two teams is fine.
- **Workspace:** set by `biorouter-crew start --name lab` when a state directory is initialized, changed by the
  host-only `workspace.rename {name}`, and returned by `hello`. Uniqueness per host account is best effort: `start`
  (before it spawns the server, `broker.rs:2765-2788`) and `workspace.rename` probe the host's own sibling runtime
  sockets and refuse a name a running sibling already uses. A legacy workspace with no name is displayed as "{host
  display name}'s workspace".
- **Rename authority:** `team.rename` the team creator, `channel.rename` the current owner, `workspace.rename` the host
  (`manager()`). A rename keeps the ID; history, invitations and grants are unaffected; old names are freed
  immediately; archived channels keep their names reserved. Pending invitations show the target's current name.

### The existence oracle

- **What leaks.** Workspace-unique team names and team-unique channel names let a member learn that a name is taken,
  including a team they are not in or a restricted channel they cannot see.
- **What the first draft got wrong.** It said probing needs "successful creates or renames, which are journaled,
  attributed and quota-limited". A refusal returns before the dedupe insert and the commit
  (`broker.rs:860-877`), so it is free and silent, and `name_conflict` computed against hidden objects was a passive
  oracle (SR6).
- **Bounds (D5).**
  - The refusal text is byte-identical whether or not the colliding object is visible: *"A team with this name, or
    one that looks like it, already exists in this workspace. Choose a different name."* It carries no ID, creator,
    member count or colliding spelling.
  - The broker keeps an in-memory, per-actor count of collision refusals; after 10 in 10 minutes it answers a generic
    "Too many name attempts. Try again later." and the host's snapshot shows a per-actor count.
  - `name_conflict` is computed only among objects the viewer can see; the owner of a hidden duplicate still sees
    their own flag.
  - The create dialogs carry one consequence line: "Everyone in this team can tell whether a name is taken. Keep
    identifiers out of names." In a biomedical lab a restricted channel name can carry an identifier.
- **Workspace names** are visible to any node user who probes the host's socket with `hello`, which is already
  unauthenticated (`broker.rs:687-689`). The host setup screen says: "Anyone who can sign in to this server can see
  the workspace name."

### Legacy journals

- Nothing is rewritten and replay never rejects old names; validation applies only to new creates and renames.
- The snapshot adds `name_conflict: true` (visible-only, above) and `name_invalid: true` for names that break the new
  rules, plus a sanitized `display_name` ("Untitled team" or `untitled` for an empty result). `name` stays raw for
  authority checks and audit. Renaming one side of a duplicate to a free name clears both flags. A second `general`
  is a legacy duplicate like any other. Two active principals sharing a username: see D3.
- **The fixture is generated in the test**, not checked in: `recovered_state` requires `host_uid == geteuid()` and
  `private_file` requires mode 0600, `nlink == 1` and the current owner (`broker.rs:137-155`, `:491-494`), which a
  checked-in file cannot satisfy. The test bootstraps normally and appends checksummed delta records carrying the
  legacy shapes, including the `sequence` patch (FR5).

## Selectors and the resolver

**Grammar**, shared by the CLI and typed GUI fields:

| Selector | Kind | Resolves among | Match |
|---|---|---|---|
| `@bob` | Person | Active principals of this workspace | Exact canonical username; a case-only difference is refused with "did you mean @bob?", never resolved silently |
| `@bob` with `--former` (CLI) or the member list (GUI) | Former person | Inactive principals in the target's member set | Same |
| `analysis-lab`, `"Analysis Lab"` | Team | Teams the caller belongs to | Projected `handle`, else `name_key` |
| `methods`, `#methods` (quoted in shells) | Channel | Channels the caller belongs to, across teams | Same; ambiguous if in more than one team |
| `analysis-lab/methods` | Channel | The caller's channels in that team | Same |
| `"UCSF HPC"`, `bob@hpc` | Connection (local) | Saved connections | Case-insensitive name, or exact `ssh_target`; ambiguous when two saved connections share a name |
| `counts.csv` with `--from methods` | Attachment | The caller's visible blobs in that channel | Exact name; ambiguous lists sender and time |
| a UUID | Any | Any; the broker still authorizes | UUID-shaped text is **always** an ID and is never matched as a name |

**Rules.**

1. **The daemon resolves (D7).** `POST /crew/resolve {connection?: selector, selectors: [{kind?, text}]}` resolves the
   connection among saved connections, then the rest against the caller's own `workspace.snapshot`, fetched with the
   person's signed request. Candidates can therefore never include hidden teams, channels or people. The route
   requires proof of a person (`require_person`) and does not call `session_reach`, because it names a connection,
   not a chat; the privacy-guard census row for `routes/crew.rs` is unchanged. Sessions are never resolved by title.
2. **Typed errors.** `unknown_name {kind, text}` returns no candidates, so it cannot become an oracle.
   `ambiguous_name {kind, text, candidates: [label]}` lists only caller-visible objects with disambiguating labels.
   Nothing is guessed.
3. **Authority-bearing inputs accept only `@username` or an ID for people**, never a display name: invite, remove
   member, ownership offer, offboard, admit. A malicious nickname "Bob Lee" cannot resolve to someone while the real
   Bob still displays as `@bob`. Display-name matching exists only in pickers and autocomplete, where each row shows
   `@username` before the click.
4. **The broker wire is unchanged for mutations**: they still send IDs. The resolve-to-commit window is closed for a
   **stale** snapshot by the generation IDs: an offboarded `@bob` is refused as inactive by `invitation.create`
   (`broker.rs:1739-1742`), and `channel.transfer` gains the same active check (FR18). A **tampered** snapshot (an
   attacker on the bridge path rewriting `@carol` to Mallory's ID) is not closed by generations (SR3). Every
   person-targeted mutation (`invitation.create`, `membership.revoke`, `channel.transfer`, `enrollment.revoke`)
   therefore gains an optional `expected_username`; the client sends the `@username` the person confirmed, the
   request is signed by the person's device key so it cannot be altered in the path, and the broker refuses
   `target_mismatch` when the principal's username differs. Full response authentication is follow-up FU7.
5. **The CLI keeps no current-team state.** `#general` is ambiguous across teams and says so; `--team` supplies
   context. The index selectors of the first draft (`latest`, `2`) are dropped; the daemon computes `latest` from its
   run ledger where a command needs it (FR8).
6. **Shell safety (FR7).** `#` starts a comment in bash and in zsh with `interactivecomments`, and `name/#x` is a glob
   operator under zsh `EXTENDED_GLOB`. The `#` is therefore optional everywhere, the qualified form is
   `analysis-lab/methods`, and every `cli-guide.md` example that uses `#` quotes it. `@bob` is safe in both shells. A
   leading `@` always selects a person, is stripped exactly once, and never splits an SSSD name such as
   `bob@ad.ucsf.edu`; connection selectors never start with `@` (SR14).
7. **`component()`** (`cli/output.rs:145-155`) gates only URL path segments. Only `--connection NAME` changes: it now
   goes to the resolver. The same character check still applies to resolved IDs (FR8).

## Invitations and joining

### Team and channel invitations by `@username` (S1)

- **Inviter:** `invites create @bob --team analysis-lab`. The GUI picker shows display name and `@username`, hides
  existing members and pending invitees, and for channel invitations offers only team members (the broker already
  requires that, `broker.rs:1750-1757`). The daemon resolves `@bob`; the broker call is
  `invitation.create {kind, target_id, principal_id, expected_username}`.
- **Invitee view.** The broker enriches each invitation it projects: `target_name`, `team_name` (channel invitations),
  `inviter {username, display_name}`, `expired`. Only the invitee and the inviter receive the invitation
  (`broker.rs:1342`), and naming the target to its intended invitee is the purpose of an invitation. Expired
  invitations are omitted for the invitee and marked for the inviter. The TypeScript `Invitation` type gains these
  fields and drops the fictional `status` (`crewApi.ts:52-58`).
- **Generation binding** is already correct and gains a regression test: acceptance requires
  `i.principal_id == actor` (`broker.rs:1783`), so a re-enrolled `@bob` cannot accept an invitation issued to the old
  one.

### Joining a workspace (S3a)

**Host, once per workspace.** In Host a workspace, enter `lab`. One command in a copyable box:
`~/.local/bin/biorouter-crew start --name lab --bootstrap-key <key>`. From S2a the command prints the workspace's
invitation line, which the host pastes back; the daemon parses it and pins the workspace exactly as it pins a
joiner's. Create workspace is the existing `auth.bootstrap`, with the bootstrap-key pin unchanged
(`broker.rs:978-984`).

**Host, per person.** Invite people → type `@bob` → **Invite**. The broker canonicalizes the name (below) and records
a pending join. The result shows `@bob · Bob Lee (name on the server account) · invited` and an invitation message to
send, with Copy:

```text
Join lab on Crew.
In Biorouter, open Crew, choose Join a workspace, and paste this whole message.
brcrew1:eyJ2IjoxLCJ3b3Jrc3BhY2VfaWQiOiIuLi4ifQ
```

**Joiner (Bob).**

1. Join a workspace → paste the message. The daemon parses it (preview) and shows the workspace name, the host, the
   server, the workspace's privacy mode and institution, and the fingerprint. "Your username on hpc.ucsf.edu" is
   prefilled; "You'll join as Private · ucsf" with Change (D9). Port, identity file, jump host and manual workspace
   details sit under Advanced.
2. **Join lab** saves the connection pinned exactly as the invitation says and connects. SSH authentication and MFA
   run as today; `hello` is verified against the pinned workspace key.
3. Bob's daemon asks `enrollment.pending` (pre-authentication). When invited, Bob's screen shows "Alice Chen (@alice)
   invited you to lab. Send Alice this code: **7QK2-M9XA-3JTP-WZ4D**" in a copyable box. **His own daemon computes the
   code from his saved device key; nothing the broker returns can change it.**
4. Alice's sidebar shows "Waiting to join · `@bob`". She pastes the code into **Let @bob in**, which sends
   `enrollment.approve {username, code}`.
5. Bob's daemon, polling `enrollment.pending` while the screen is open, sees `approved` and sends `auth.join`. The
   broker binds Bob's key only if its device code equals the approved code. Bob is in. The order can also run the
   other way: if Alice pastes first, Bob joins as soon as his screen is open.

Machine strings a joiner copies by hand: **zero**, down from seven. Values carried between people: one invitation
message (host to joiner) and one 16-character code (joiner to host), both copied with one click.

### The invitation

```text
brcrew1:<base64url(JSON)>          at most 4 KiB after decoding
JSON = {
  "v": 1,
  "workspace_id": "<uuid>",  "workspace_public_key": "<64 hex>",
  "socket_path": "/tmp/crew-<uid>-<32 hex>/broker.sock",  "owner_uid": 1000,
  "workspace_name": "lab" | null,
  "host_username": "alice",  "host_display_name": "Alice Chen" | null,
  "mode": "private" | "public",  "institution_id": "ucsf" | null,
  "ssh_host": "hpc.ucsf.edu",  "ssh_port": 22 | null,  "proxy_jump": "gateway.ucsf.edu" | null,
  "invitee_username": "bob" | null
}
```

- **Provenance.** The host's daemon builds it from the host's own verified connection (the four pinned fields), the
  last verified snapshot (name, mode, institution, host username) and `ssh -G` output for the host's SSH target
  (hostname, port, jump host; never a local alias). `biorouter-crew start` prints the same line for the host. The
  workspace key therefore still comes from the host over a human channel, exactly as the protocol contract requires
  today (`protocol-contract.md:110`); there is no trust-on-first-use change.
- **Parsing (D9).** Only the daemon parses it, in `biorouter_crew::invitation::parse`, exposed as
  `POST /crew/connections/from-invitation {invitation, preview?, username?, mode?, institution_id?, advanced?}`. The
  parser extracts the first `brcrew1:` token from pasted text, accepts the legacy `biorouter-crew status` JSON too,
  validates every field as `validate_connection` does today, and refuses an unknown `v`. Preview returns the parsed
  summary without saving. The CLI (`biorouter crew connections join-invitation -`) and the GUI share it.
- **Labels are not authority.** `workspace_name`, `host_display_name` and the SSH hints are display metadata and
  defaults; `hello` must verify against the pinned workspace key before anything is trusted, and the broker enforces
  the institution match for protected context (`broker.rs:1084-1115`).
- **Privacy (D9, FR10, SR7).** `mode` and `institution_id` prefill the joiner's connection and are shown for
  confirmation. A Private save therefore always has an institution, which closes the save-before-institution dead
  end of the first draft (FR3.4). The joiner can change the mode; a mismatch line appears only when the choice
  differs from the workspace's.
- **Integrity, not secrecy.** An invitation reveals nothing a node user cannot learn from `hello`, plus the host's
  chosen labels. Forwarding it to the wrong person gains them nothing: they still need an account the host invited
  and the host's approval of their device code.

### The device code

```text
device_code(workspace_id, W, K) =
  Crockford-base32( SHA-256( "biorouter-crew-device-code-v1\0" ‖ workspace_id ‖ "\0" ‖ W ‖ K )[0..10] )
  → 16 characters, displayed 7QK2-M9XA-3JTP-WZ4D
```

`W` is the 32-byte workspace public key the joiner pinned, `K` the joiner's 32-byte device public key. Input is
normalized by removing spaces and hyphens, upper-casing, and mapping `I`/`L` to `1` and `O` to `0`; `U` is refused.

- **Computed at both ends, shown only by the joiner.** Bob's daemon computes it from its saved key and pinned `W`. The
  broker computes it from the claimed key and its own `W` when checking `auth.join`. No broker response contains a
  code (a schema guard pins this), and the GUI never renders a code it did not compute or receive from a person.
- **Why 80 bits.** A relaying attacker must present a key `K_m` whose code equals the code Bob's own screen shows,
  which is a second preimage on 80 bits: infeasible within an invitation's lifetime. There is no claims list, voiding,
  claim cap or per-claim nonce to get wrong (FR1 fix A).
- **Why `W` is inside.** If an invitation were tampered in transit to carry another workspace key, Bob's code would be
  computed over the substituted key and the real broker's check would fail, so a tampered invitation cannot admit
  anyone. This is defence in depth only: an 80-bit code over an attacker-chosen `W'` and `K_m` would fall to a
  birthday search of about 2⁴⁰, which is why S4 (where `W` could come from discovery) must use the two-sided
  commit-then-reveal code instead (D10).

### Broker protocol (S3a)

**State.** A top-level field serialized only when non-empty (D11):

```rust
#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
pending_joins: BTreeMap<String /* uid, decimal */, PendingJoin>,

pub struct PendingJoin {                         // lib.rs, pub for the daemon's tests
    pub join_id: String,                         // 128 random bits; a re-invite mints a new one
    pub uid: u32,
    pub username: String,                        // canonical
    pub full_name: Option<String>,               // validated; label only
    pub inviter_id: String,                      // host principal UUID
    pub existing_principal_id: Option<String>,   // Some = add a device
    pub generation: Vec<String>,                 // principal IDs holding this UID or username at invite time
    pub approved_code: Option<String>,           // normalized 16 characters
    pub created_at: u64,
    pub expires_at: u64,                         // created_at + 24 h
}
```

The first join emits a top-level `Set [pending_joins]` whose parent is the root; the last removal emits
`Remove [pending_joins]`. Both replay on old and new brokers, so there is no backfill, no upgrade record and no write
when a broker opens (FR2). An older broker deserializes `State` without `deny_unknown_fields` and ignores the map.

**Account directory seam.** The free function `username(uid)` (`broker.rs:105-125`), called on every authenticated
request (`:778`, `:1128`), becomes a `Directory` trait with `by_uid(uid) -> Account {uid, name, full_name}` and
`by_name(name) -> Account`, backed by `getpwuid_r`/`getpwnam_r`. Tests inject a fake through
`Broker::open_with_directory(root, key, Box<dyn Directory + Send>)`, compiled only with the crate's `test-seams`
feature (enabled through a dev-dependency self-reference). The shipped binary (`--bin biorouter-crew`) never enables
it (FR4).

**Canonicalizing a typed name (SR9).**

1. Strip one leading `@`.
2. Require 1–256 bytes, no NUL, control characters, whitespace, `/` or `:`, and not all digits (glibc's `getpwnam("1001")`
   looks up a name while `getent passwd 1001` looks up a UID).
3. `a = by_name(input)`, `b = by_uid(a.uid)`; require `a.name == b.name`, else refuse
   `identity_ambiguous: @X is an alias on this server; invite @<b.name>`.
4. Require `input == b.name` exactly; a case-only difference is refused with the canonical spelling, never accepted.
5. Revalidate `b.name` with the same character rules, since it comes from NSS.

**Methods.** The `manager()` check always runs before any NSS call.

| Method | Caller and gate | Params | Result | Journal |
|---|---|---|---|---|
| `enrollment.invite` (new form) | Signed human; manager | `{username, add_device?: bool}` | `{username, full_name, add_device, join_id, expires_at}` | Mutation |
| `enrollment.invite` (legacy form) | Unchanged (`broker.rs:1563-1611`), plus the key-already-a-device check | `{uid, public_key, existing_principal_id?}` | Unchanged, including the token | Mutation |
| `enrollment.approve` | Signed; manager | `{username, code}` | `{approved: true, username}` | Mutation; replaces an earlier approved code only on an explicit re-approve |
| `enrollment.cancel` | Signed; manager | `{username}` | `{cancelled: true}` | Mutation |
| `enrollment.pending` | **Pre-authentication**, like `hello`; kernel UID only; built from state with no NSS call | `{}` | `{invited: false}`, or `{invited: true, join_id, workspace_name, inviter: {username, display_name}, add_device, approved, expires_at, last_refusal?: "code_mismatch"}` | Read |
| `auth.join` | **Pre-authentication**, like `auth.enroll`; signed by the claimed key over an `auth.challenge` nonce | `{public_key, join_id}` | `{principal: {username, display_name}, device_id, workspace}` | Commits directly: operation `auth.join`, actor `uid:<n>` |
| `profile.suggest` | Signed; actor only | `{}` | `{full_name?}` | Read |
| Snapshot addition (manager only) | — | — | `pending_joins: [{username, full_name, add_device, approved, created_at, expires_at, mismatched_attempts}]`, never a code, key, UID or join ID | — |

`enrollment.pending` and `auth.join` join the pre-authentication dispatch beside `hello` and `auth.*`
(`broker.rs:687-695`); `enrollment.pending` joins the read set. The `hello` capability `join_by_name_v1` is advertised
only by a broker built with the `join-by-name` feature (D17).

**New-form `enrollment.invite` checks, in order:** `manager()`; canonicalize; then an active principal on this UID
with the same username requires `add_device = true` (else "@bob is already a member; choose Add device"); an active
principal on this UID with a different username is refused `identity_mismatch` (remove first); an active principal
with the same canonical username or skeleton on a different UID is refused `identity_conflict` (D3); a pending join on
the same username for a different UID is refused. The join records the `generation` and replaces any earlier join for
the UID (new `join_id`).

**`auth.join` checks, in order.** Any refusal writes nothing.

1. A pending join exists for the kernel UID, is not expired, and has the given `join_id`.
2. `by_uid(uid).name == join.username` (guards renames and recycled UIDs).
3. The principal state still matches: the set of principal IDs holding this UID or username equals `generation`; for
   add-device, `existing_principal_id` is active with the same UID and username; otherwise no active principal holds
   the UID or the username.
4. `verify(uid, conn, req, public_key)` (`broker.rs:930-974`) proves possession of the key with a single-use,
   60-second, socket-bound nonce.
5. The key is not already a device (SR11).
6. `approved_code` is set and equals `device_code(workspace_id, W, public_key)`. Otherwise refuse `code_mismatch`,
   set the in-memory `last_refusal` for `enrollment.pending`, and increment the join's in-memory
   `mismatched_attempts`, which the host sees as "A device with a different code tried to join as @bob" before the
   code field (SR1's "warn before the code").
7. D3 still holds.

On success: create the principal (nickname = username, D2) or select the existing one for add-device, insert the
device with `added_via: "invitation_code"`, remove the join, commit.

**Lifecycle (SR10).** `enrollment.revoke` purges pending joins for the target's UID and username. Expired joins are
pruned on every mutation, so they never count against the cap of 100. Completing the legacy token path for a UID
removes its pending join, and a successful `auth.join` removes any legacy enrollment for the UID. A re-invite mints a
new `join_id`, so a screen still showing an old code gets `code_mismatch` and re-reads its status.

**Key already a device (SR11)** is refused at `auth.join`, at legacy `auth.enroll` (`broker.rs:1012-1019` inserts
without checking today) and at legacy `enrollment.invite` (`:1593-1597`).

**NSS under the broker lock (SR12, FR14).** Every request runs under one mutex (`broker.rs:2643-2646`), and NSS can
block on LDAP. Names are looked up on submit, never per keystroke; results are cached for 30 seconds; the snapshot
path makes NSS calls only for the host's `account_stale` pairs; `enrollment.pending` and `auth.join` make exactly the
one `by_uid` call of check 2.

### What keeps verification material, and the fallbacks

- **Host bootstrap** keeps the bootstrap-key pin, shown once inside the single start command. Having the desktop run
  `start` over the saved SSH connection is follow-up FU3.
- **Legacy token path** (Advanced, "Invite someone using an older version of Biorouter"): the joiner's screen shows a
  join request (username and device key) to send; the host enters the joiner's user ID (the host runs `id -u bob` on
  the server), and the unchanged legacy `enrollment.invite {uid, public_key}` returns a token in a secret copy box. It
  is also what every joiner sees when the host's broker lacks `join_by_name_v1`. It is removed **two releases after
  S3a ships enabled**, with a release note (FR11); "when all desktops advertise it" was unmeasurable, because desktops
  advertise nothing to brokers.
- **Workspace fingerprint** stays visible at the join confirmation, grouped from `workspace_key_fingerprint`
  (`3F2A 9C1E 77B0 D4E1`), with Copy.

### Security analysis

**Attackers.** **M1:** another node user. **M2:** a process running as the **joiner's own UID** on the broker node: a
compromised package, a batch job, a notebook kernel, or a Crew worker that wrote into a work folder (SR4). It can
replace the joiner's `~/.local/bin/biorouter-crew`, wrap it through shell startup files, or ptrace it, and so can
**relay, drop, reorder and substitute** every frame between the joiner's daemon and the broker. **M3:** a non-host
member probing names. **M4:** someone who sees or hears the code. **M5:** a process running as the host's UID, trusted
by the plan (`plan:124`).

| Property | How the design keeps it | Holds? |
|---|---|---|
| Kernel-derived identity: every actor is `SO_PEERCRED` UID → device → principal with `username == getpwuid(uid)` (`broker.rs:777-780`) | Unchanged for every request. The invited UID and canonical username are bound at invite and rechecked at join. Names never authenticate a request. | Yes |
| UID recycling and generation | The join binds UID, canonical username and the principal generation; all are rechecked at join. A renamed account, a recycled UID with a new name, or a new UID reusing the name fails closed. A recycled UID that also reuses the name is indistinguishable, as today; the host still chooses Add device or Remove explicitly. | Yes, stronger than today |
| **Device-key human authority; no first-writer-wins** (`plan:237-241`, I02) | M2 in the path cannot choose what Bob's screen shows: Bob's daemon computes the code from Bob's key. To be bound, M2 needs a key whose 80-bit code equals Bob's. Racing M2 claims are refused unless their key matches the code the host approved. Blocking M2 can only delay Bob. | **Yes** (the first draft failed here, SR1) |
| Replay | `auth.join` reuses `verify()`; approval and invite go through mutation dedupe; a successful join removes the pending join | Yes |
| Non-invited node user (M1) | `enrollment.pending` answers only `{invited: false}` and nothing else; `auth.join` from an uninvited UID is refused and writes nothing | Yes |
| Only the host admits people | Invite, approve and cancel are `manager()`-gated; no join happens without the host approving a code | Yes |
| No OS account enumeration (I03) | `getpwnam` is a point lookup, gated by `manager()` before the call, on submit only | Yes |
| Secrets at rest | No token in the new path; the code has integrity value only; the dedupe cache holds no secret | Improved (the legacy token cached in `dedupe` remains on the legacy path only) |
| Host impersonation | No discovery: the workspace key comes from the host's invitation over a human channel, as today | Yes, unchanged trust model |
| Tampered snapshot or status (SR3) | Person-targeted mutations carry `expected_username`, inside the person's signature; a faked `enrollment.pending` or join status can only mislead the joiner's screen, never bind a key | Bounded; full response authentication is FU7 |
| Worker becoming M2 (SR4) | D15 protects the bridge path and shell startup directories from work folders | Yes for this path |
| Existence oracles | `enrollment.invite` host only; `enrollment.pending` answers only the caller about itself; team and channel names bounded by D5; the resolver draws only from the caller's snapshot | Bounded as stated |
| Availability | M2 can block Bob's bridge so his claim never arrives. The join stays pending until it expires; the host sees mismatched attempts when M2 tries its own key. This is a denial of service by a process already running as Bob, with no access gained. | Accepted, documented |
| Shoulder-surfing the code (M4) | A code only matches the key it was computed from | N/A (integrity, not secrecy) |

Social engineering ("send me your code" from someone pretending to be Bob) is equivalent to "send me your key" today
and is out of scope in both flows.

## Discovery and short codes (S4, deferred)

Finding a workspace from the host's `@username` and reading an 8-digit code aloud remain goals, but not in this
campaign. When S4 is built it must satisfy all of these, and it needs its own adversarial review and a maintainer's
explicit decision on moving the workspace key's provenance from a human channel to a scan of the node (SR2, FR3):

- **Two-sided commit-then-reveal code** (SR1 fix B). The desktop commits `H(W ‖ workspace_id ‖ join_id ‖ K ‖ R_d)`
  (signed), the broker returns `R_b` signed by `W`, the desktop reveals `R_d`, and **both** ends compute
  `u64(SHA-256("biorouter-crew-join-code-v2\0" ‖ W ‖ workspace_id ‖ join_id ‖ uid ‖ K ‖ R_d ‖ R_b)[0..8]) mod 10⁸`.
  One commitment per key per join; unopened claims count toward a cap of 3; the desktop persists `(join_id, R_d, R_b)`
  and re-rolls only on an explicit Start over, at most three times; codes expire 30 minutes after opening; no response
  carries a code; only this variant ships (capabilities are unsigned before `hello` v2, so a weaker variant could be
  forced).
- **Discovery's owner checks run in a binary the joiner's UID controls**, so its results are trust-on-first-use
  against the joiner's own account, verified end to end only by the code above, which binds `W`.
- **Cluster merging by the OpenSSH-verified host key as well as `node_id`** (`core/mod.rs:880-897` merges by `node_id`
  alone, which a spoofed `hello` chooses); a merge only ever raises a connection to Private.
- **Never re-pin.** Discovery runs only for unpinned connections; a candidate whose `workspace_id` matches a saved
  connection with a different key is refused; a failed `hello` on a pinned connection shows the identity-change state.
- **Bounded scan**: at most 64 prefix matches, and capped output lines and bytes before parsing.
- **Version skew**: `discover` runs the joiner's own binary; an older binary exits with a usage error whose stderr is
  discarded today (`core/transport.rs:113`), so the daemon must detect that and fall back to the invitation.
- **Unverified connections** must use optional workspace fields excluded from alias merging until pinned
  (`core/mod.rs:618-622` would merge two empty `workspace_id`s), or not be persisted.
- **Test seams**: a filesystem-root seam (`validate_socket` hard-codes `/tmp`) and a fake peer credential.

## Machine IDs stay internal

- **GUI default paths show no UUIDs, 64-hex values or numeric UIDs.** Copy … ID items exist on a person, team,
  channel, message, attachment (and SHA-256), task and invitation; Connection settings → Advanced shows the workspace
  ID, fingerprint, socket path, host user ID, device ID and cluster ID. No other surface shows them.
- **Error text.** The daemon maps broker `code` values to sentences and never forwards the JSON envelope; fallbacks
  say "this workspace", "this team" or "this channel".
- **CLI.** Human output uses type-specific formatters; `--show-ids` appends IDs; `--output-format json` and
  `stream-json` are unchanged and may carry IDs. A request ID is printed only in a retry hint.
- **Model.** IDs remain in tool arguments because the model must pass them; every result also carries names. The tool
  description and the owned-task system prompt say: "Refer to people as *Display name (@username)* and to channels as
  *#name*. Never quote IDs to people."

## Agents and models

- **Labels at admission (D14).** `agent_connections` (`core/mod.rs:1194-1201`) runs on the worker path, and workers
  cannot call `workspace.snapshot` (`broker.rs:739-755`). The labels (`you`, workspace, destination and source channel
  names) are captured when `checked_run_admission` fetches its human-signed snapshot under the person's action
  (`core/mod.rs:1344-1348`) and stored in the scope as display-only, stale-able data. `RunAdmission.context` carries
  the same labels.
- **Task title after admission.** `create_run_session` titles the session "Crew task" before admission
  (`routes/crew.rs:516-528`); the title becomes `Crew · #methods · <prompt excerpt>` once admission names the
  channel.
- **Public providers (D13).** Owned-task and granted-chat contexts use a person's display name only when that person
  set or confirmed it (a nickname differing from the username is treated as set), and `@username` otherwise, so a
  legal name taken from the server account never reaches a public provider by default.

## Code changes by layer

### Broker and shared library (`crates/biorouter-crew`)

| Slice | Change | Where |
|---|---|---|
| S1a | `clean`, `name_key`, the display-name validator with the generated default-ignorable table, `sanitize_display_name` | `lib.rs` |
| S1a | Snapshot: `former_principals`, invitation enrichment, `host_principal_id` at projection, per-principal `display_name`, `actor.devices`, `account_stale` (host only) | `read_workspace_snapshot`, `broker.rs:1308-1344` |
| S1a | Message results: the `people` map and runtime-filtered `channel_names` | `message_wire` callers, `broker.rs:1226-1230`, `:1284-1292`, `:1345-1388`, `:1406-1424` |
| S1a | D3 at legacy `auth.enroll` and legacy `enrollment.invite`; `Device.added_at/added_via`; key-already-a-device at both legacy binds | `broker.rs:975-1030`, `:1563-1611` |
| S1a | `profile.update` validation and `nickname: null` reset; `profile.suggest`; `expected_username` on person-targeted mutations; `channel.transfer` active check | `broker.rs:1545-1562`, mutation handlers |
| S1a | `hello.capabilities` gains `human_names_v1` | `broker.rs:911` |
| S2a | Team, channel and workspace validators; `strip_ignorable`; UUID-shaped refusal | `lib.rs` |
| S2a | `team.create`/`channel.create` uniqueness; `team.rename`, `channel.rename`, `workspace.rename`; visible-only `name_conflict`, `name_invalid`, projected `handle`; the per-actor collision rate limit | `broker.rs:1506-1530`, `:1662-1727`, snapshot |
| S2a | `Workspace.name` (`#[serde(default)]`); `start --name` validated before the spawn, sibling probe; `start` prints the invitation line; `hello` returns `name`; `hello` signature v2 over `[workspace_id, host_uid, nonce, W, node_id, mode, institution_id, policy_epoch, name, capabilities]` beside v1; capability `unique_names_v1` | `lib.rs:84-92`, `broker.rs:879-913`, `:2763-2819`, `main.rs:32-41` |
| S2b | `skeleton_key`, `Identifier_Status` and restriction-level checks via `unicode-security`; skeleton checks for display names and D3 | `lib.rs`, `Cargo.toml`, `Cargo.lock` |
| S3a | `PendingJoin`, `device_code`, `invitation::{parse, encode}` | `lib.rs` |
| S3a | `pending_joins` state; `Directory` seam and `open_with_directory` (`test-seams`); canonicalization; `enrollment.invite` (new form), `approve`, `cancel`, `pending`; `auth.join`; lifecycle rules; manager-only snapshot projection; capability `join_by_name_v1` under `join-by-name` | `broker.rs` hooks, with the join logic in `broker/join.rs` compiled only with the feature |
| S3a | D15 protected paths | `remote.rs:47-60` |

### Daemon core (`crates/biorouter/src/crew`)

| Slice | Change |
|---|---|
| S1a | Broker capabilities in an in-memory map on `CrewManager`, refreshed in `connect_locked`, never in `Connection` (so `connection_binding` stays stable, FR9) |
| S1a | Admission-time labels in the scope (D14); `agent_connections` emits them |
| S2a | `verify_workspace_identity` accepts `hello` v1 and prefers v2; with v2, name, mode, institution and capabilities are trusted for display |
| S3a | `connection_from_invitation(preview)`, `invitation_for(connection)` (host), `join_status(id)` (unsigned pre-authentication `enrollment.pending`), `join(id)` (`auth.challenge` + `auth.join` signed with the saved device key), `device_code(id)`; the `signed_request` enrollment guard (`core/mod.rs:1092-1097`) extended to `auth.join` so the generic request path can never send it |

### Daemon routes, OpenAPI and TypeScript client

| Slice | Route | Notes |
|---|---|---|
| S1b | `POST /crew/resolve` | Connection-independent; `require_person`; typed `ToSchema` request and response |
| S1a | Observation `state` frames gain `labels` | Computed by the same code as the resolver |
| S1a | Task title after admission; naming guidance in the owned-task system prompt and the Crew tool description | `routes/crew.rs:527`, `:586`; `agents/crew_extension.rs` |
| S3a | `POST /crew/connections/from-invitation`, `GET /crew/connections/{id}/invitation`, `GET` and `POST /crew/connections/{id}/join` | `require_person`; typed schemas; no `session_reach` |
| S3a | Host actions (`enrollment.invite`, `approve`, `cancel`) | The existing `/request` pass-through, which refuses only `run.` and `worker.` |

After each route change, register it in `crates/biorouter-server/src/openapi.rs` and run
`just generate-openapi && cd ui/desktop && npm run generate-api`. CI's `scripts/check-openapi-schema.sh` blocks drift.
The snapshot is forwarded as `Value`, so the hand-written TypeScript `Principal`, `Invitation` and `Snapshot`
interfaces are edited by hand.

### CLI (`crates/biorouter-cli/src/commands/crew`)

| Slice | Change |
|---|---|
| S0/S1a | `output.rs`: type-specific human formatters for messages, members, teams, channels, invites, connections, tasks, grants, privacy and files; names quoted with Unicode isolates; `--show-ids`; golden tests with no UUID or 64-hex string |
| S1b | `args.rs`/`mod.rs`: every object argument accepts a selector resolved through `POST /crew/resolve`; `#` optional; `--connection NAME`; `invites create @bob --team T \| --channel T/c`; `invites accept` implicit when one is pending; `remove-member c @bob [--former]`; `ownership offer c @bob`; `history\|search\|watch c`; `channels mark-read c`; `enroll revoke @bob --confirm @bob` (required without a TTY, since `--approval-key-stdin` consumes stdin; `enroll revoke <ID>` stays prompt-free for scripts, FR16) |
| S2a | `teams rename T NEW`, `channels rename T/c NEW`, `workspace rename NEW`; create commands echo the stored name |
| S3a | `connections join-invitation -` (reads the pasted message on stdin), `connections invitation [--for @bob]`, `crew join` (prints the code and waits), `enroll invite @bob [--add-device]`, `enroll pending`, `enroll approve @bob CODE`, `enroll cancel @bob`; the legacy `enroll invite --uid --public-key` and `enroll accept` stay, hidden in help and marked deprecated |

A new CLI talking to a stale shared daemon gets 404 from new routes and says "Restart the shared Biorouter daemon to
use names; IDs still work." It calls `/crew/resolve` only for input that is not UUID-shaped.

### GUI (`ui/desktop/src/components/crew`)

The GUI's layout and copy are specified in the [UI redesign specification](ui-redesign-spec.md). This design owns the
data it shows: `PersonName` and `personLabel()` implementing the display rule and preferring daemon labels; no ID in
any default path; invitations by name; host detection by `host_principal_id`; the create dialogs' live slug preview
and name-refusal wording (S2); the join and admit screens with the invitation, the privacy confirmation, the
locally computed code and the host's paste field (S3a).

## Backward compatibility

### Journals

- **Additive state only.** `Workspace.name`, `Device.added_at`, `Device.added_via` are `#[serde(default)]`;
  `pending_joins` is additionally `skip_serializing_if` empty. Name keys, flags, handles and sanitized names are
  computed, never stored. The record format and checksum chain are untouched.
- **No materialization step** (FR2, SR13). The first draft's single-patch upgrade record would have failed replay with
  `journal_corrupt: state sequence mismatch`, because replay checks the sequence after every record and the record
  lacked `Set [sequence]`; computing it through `commit()` would never have emitted it at all, because both sides
  serialize the defaulted empty map.
- **Downgrade** is tested with the **previous release's broker binary** against a journal the new broker wrote
  (join added and removed), not with the new code minus a backfill (SR13).
- **Legacy data** that breaks the new rules stays valid on replay; the generated fixture proves it.

### Wire and version negotiation

Every result gains fields; nothing is removed. `PROTOCOL_VERSION` stays 1 (`lib.rs:6`); bumping it would refuse every
old client. New methods are offered only after a capability check, but `expected_username` is sent regardless: a
capability is unsigned without `hello` v2 and can be stripped (last row), and an older broker ignores the field.

| Combination | What happens | Handling |
|---|---|---|
| New daemon, old broker | No `human_names_v1`, `unique_names_v1` or `join_by_name_v1`; new methods fall through to authentication and fail closed | "Unknown member" and "Invitation from {inviter}"; Rename hidden; the legacy token path |
| Old daemon or client, new broker | Legacy `enrollment.invite {uid, public_key}` and `auth.enroll` unchanged. After S2a, colliding team names are refused with plain text and channel names canonicalized; after S1a, an invalid nickname is refused and expired invitations disappear for the invitee | Recorded in the release notes |
| New broker, old joiner desktop | The joiner has no invitation paste | The host's invite result offers "Bob's Biorouter is older? Invite with a token instead" |
| Host broker upgraded, old joiner binary | The bridge is unchanged, so the invitation and device code work; S3a needs no joiner binary upgrade | — |
| New CLI or renderer, stale shared daemon | `SaveConnection` refuses unknown fields; new routes return 404 | The CLI calls `/crew/resolve` only for non-UUID input and explains a 404; the GUI hides name-only affordances |
| Spoofed capability on the bridge path | Capabilities are unsigned before `hello` v2 | Adding one fails closed on an old broker; stripping one downgrades to the legacy path, which is still safe |

### Local configuration

`Connection` gains nothing persisted for capabilities (D12). New optional `SaveConnection` fields carry
`skip_serializing_if = "Option::is_none"`, so the prepared-identity idempotency hash over the serialized input
(`core/mod.rs:705`, `:714`) is unchanged for saves that do not use them (FR17). `connections.json` stays readable in
both directions.

## Tests

### Broker (`crates/biorouter-crew`)

A shared harness, `tests/support/mod.rs`, replaces the per-file helpers (`bootstrap`, `signed`, `enroll_user`,
`request`) and hosts the generated legacy journal and the fake `Directory`.

**S1a.** `former_principals` includes only inactive principals referenced by visible objects. The invitee's snapshot
carries `target_name`, `team_name` and `inviter`; a non-invitee's carries none; expired invitations are hidden for the
invitee and marked for the inviter. Message results carry the `people` map for active and former authors, and
`channel_names` never names a channel the actor cannot read. D3: a second active `@bob` (or `@Bob`) on another UID is
refused with `identity_conflict`, and a generated legacy journal with such a pair flags `account_stale` in the host's
snapshot only. Display names: U+202E, U+2066, U+200B, U+200D, U+FEFF, U+FE0F alone, U+3164, a control character, `@`,
fullwidth `＠`, empty, 65 characters, and `alice` when `@alice` exists (active or former) are refused; `李明 Li Ming`
and the caller's own username are accepted; legacy violators fall back to the username. A team invitation issued to
the old `@bob` cannot be accepted by a re-enrolled `@bob`. `expected_username` mismatches are refused on each
person-targeted mutation. `channel.transfer` refuses an inactive successor. `nickname: null` resets. `profile.suggest`
returns only the caller's own validated full name.

**S2a.** The team uniqueness table: `Lab`/`lab`/`LAB`; `Analysis Lab`/`analysis-lab`/`ANALYSIS_LAB`/`Analysis.Lab`;
fullwidth `Ｌａｂ`; `Lab` + U+FE0F; a self-rename changing only case allowed. Channel canonicalization: `Data Analysis`
stored as `data-analysis`, `data_analysis` colliding with it, the same slug in two teams allowed, an archived channel
still reserving its name, `general` reserved. UUID-shaped team and channel names refused. The collision refusal is
byte-identical for a visible and a hidden colliding team and names no ID, creator or spelling; the eleventh refusal in
ten minutes gets the generic answer; `name_conflict` never reflects a hidden object. Rename authority for creator,
owner and host; anyone else refused. Workspace name format, host-only rename, and the sibling probe with a fake
runtime directory. The generated legacy journal: duplicate `Lab`/`lab` teams, a second `general`, a whitespace-only
team name, a U+202E nickname and two active `@bob`: replay succeeds with the sequence and checksums intact, flags set,
projections sanitized, a new `LAB` refused, renaming one duplicate clears both flags, and a re-open after that
mutation replays. `hello` v2 verifies over every listed field and v1 still verifies.

**S2b.** `Аnalysis` (mixed script) refused; `anaIysis` beside `analysis` refused; the skeleton collision rule for
display names and D3.

**S3a.** Canonicalization with the fake `Directory`: an NSS alias refused; `Bob` for canonical `bob` refused with the
canonical spelling; an all-digit name refused; an unknown name refused; a non-manager's invite refused **before** the
`Directory` is called (call count 0). `enrollment.pending` from a non-invited UID returns exactly `{invited: false}` and
makes no NSS call; `auth.join` from it is refused and the journal length is unchanged. A wrong code is refused,
records nothing, sets `last_refusal` and increments `mismatched_attempts`. Renamed, recycled and reused accounts are
refused at join; a changed generation is refused; add-device requires `add_device` and binds the existing principal.
Expiry, cancel and re-invite (the old `join_id` refused). Restart between approve and join: the approval survives and
the join succeeds. `enrollment.revoke` purges pending joins. Key already a device refused at `auth.join` and at both
legacy binds. The legacy token path still enrolls and removes the pending join; the dedupe cache for the new methods
holds no code. A schema guard: no response contains a code field.

**The hostile-bridge harness (SR1).** A test double sits between a simulated daemon and the broker and relays, drops,
reorders and substitutes frames, holding its own key. In every ordering (attacker first, Bob first, Bob blocked,
attacker replaying Bob's frames, attacker grinding up to 10⁶ keys against the displayed code), assert that the
attacker's key is never bound and that the host's approval of Bob's desktop-computed code binds only Bob's key.

**Journal.** `skip_serializing_if` replay and the previous-binary downgrade (above).

**D15.** Each protected folder (`~/.local/bin`, `~/bin`, `~/.bashrc.d`, `~/.profile.d`, the bridge's directory and its
ancestors up to `HOME`) is refused as a work folder, and `~/crew-work/lab` is accepted.

### Core and daemon

- The resolver: grammar parsing (`@`, optional `#`, `team/channel`, quoted names, UUIDs); exact-only person matching
  with the case refusal; `general` ambiguous across teams with candidates; unknown names list no candidates;
  candidates come only from the snapshot; a display name is never accepted for an authority kind; a UUID-shaped name
  is always an ID; a disagreement between the daemon's key and the projected handle answers ambiguous.
- `POST /crew/resolve` without proof of a person is refused; the privacy-guard census is unchanged.
- `connection_binding` is unchanged by a capability refresh (FR9).
- `agent_connections` and the admission context contain a label for every ID they carry, and never come from a
  worker-path human-signed call.
- S3a: the invitation parser accepts the message, the bare line and legacy status JSON, refuses garbage, unknown
  versions and oversize input; a preview saves nothing; a Private invitation saves with its institution;
  `device_code` equals the broker's computation for the same inputs; a pinned connection whose `hello` fails is never
  re-pinned; `auth.join` is refused through the generic `/request` path.

### CLI

Golden formatter tests with no UUID or 64-hex string in text output and IDs present with `--show-ids`; JSON output
byte-identical to before apart from added fields; `args.rs` parse tests for every selector argument, with and without
`#`; an ambiguous error prints its candidates and exits non-zero; `enroll revoke @bob` without a TTY requires
`--confirm @bob`.

### GUI

A rich fixture (a former-member author, an invitation, people, channel settings, a task row, a pending join) renders
no UUID, 64-hex string or numeric UID in the default DOM, and each appears after its Copy … ID. A same-name collision
renders both people with `(@username)`. Expired invitations are hidden. The join and admit screens against a mocked
daemon: invitation preview, privacy confirmation, invited with the locally computed code, not invited, code mismatch,
expired, joined; the host's code field accepts hyphens, spaces and lower case; the host never renders a code.

### Live acceptance

On the disposable AWS fixture with three real Unix accounts and private Versa GPT-5.5 for agent steps: Alice hosts
`lab`; Bob and Carol join by invitation; a wrapper running as **Bob's** UID sits in Bob's bridge path and substitutes
its own key and relays Bob's frames; Alice admits only Bob by his code and sees the mismatched-attempt warning; Carol,
not invited, sees only "not invited"; a renamed synthetic account is refused; an agent's channel post names people and
channels and quotes no ID. Evidence follows the same-final-revision rule.

### Existing regressions

No current GUI test asserts a UUID in rendered text; CLI `output.rs` tests cover escaping only. Any test that asserted
raw field dumps is updated deliberately with the reason recorded beside it; none is deleted.
`component_accepts_stable_ids_and_refuses_terminal_or_empty_values` keeps passing for IDs.

## Slices, order and review gates

| Slice | User-visible result | Size (production + test lines, rough) | Risk | Gate |
|---|---|---|---|---|
| **S0** presentation only (daemon, GUI, CLI; no broker change) | No UUID by default: "Unknown member", "Invitation from Alice Chen (@alice)", Copy … ID, daemon-computed host detection | ~300 + ~300 | Low | Standard review |
| **S1a** broker projections and D3 | Former members by name; invitations say who and what; the `people` map; devices visible; display-name rules; `expected_username` | ~450 + ~800 | Low–moderate (projection leaks, spoofed names) | Standard review with the disclosure bounds checked |
| **S1b** resolver, route, CLI selectors and formatters, admission labels | The CLI takes `@bob`, `analysis-lab`, `methods`; agents write names | ~900 + ~800 | Low–moderate | Standard review |
| **S2a** unique names, renames, workspace name, `hello` v2 | Teams and channels cannot collide; Rename works; the workspace has a name; `start` prints the invitation line | ~600 + ~900 | Low–moderate (Unicode, oracle wording, legacy replay) | Standard review plus the generated-journal gate |
| **S2b** confusables | Lookalike and mixed-script names refused | ~150 + ~200 | Low (one new pinned dependency) | Supply-chain review of `unicode-security` |
| **S3a** join by invitation and device code | Invite `@bob`, paste one message, send one code, paste it back; no token or descriptor field on the default path | ~1,300 + ~1,600 | **High** (new pre-authentication methods, key binding, journal state) | **Mandatory** independent adversarial identity and authorization review, the hostile-bridge harness, the previous-binary downgrade test and the live three-account run, **before** the `join-by-name` feature is enabled in a release build (plan §16.5) |
| **S4** discovery and 8-digit codes | Join by host `@username` without an invitation | not sized | High | Its own design revision and adversarial review; maintainer decision on trust provenance |

S2 does not have to precede S3a: its only dependency on names was discovery, which is deferred, and S3a needs D3
(S1a) and the `Directory` seam, nothing else. Within each slice, `broker.rs` has one owner; S3a's join logic lives in
a separate module compiled only with its feature, so S1a and S2a can ship while S3a waits for its review.

## What the docs keeper must record

- **Decisions log:** D1–D17 above, with SR and FR references. D8 supersedes the enrollment steps in
  `protocol-contract.md:25`, `cli-guide.md:106-125` and the `plan:112` descriptor as the primary path; first-writer-wins
  remains forbidden (`plan:239`) and is satisfied by the host's approval of a desktop-computed device code. D10 records
  that discovery is deferred pending a maintainer decision on trust provenance.
- **Plan §16.4 wording.** Replace "Resolution uses the broker's directory of enrolled, discoverable principals (§5),
  never OS account enumeration" with: "Team and channel invitations resolve `@username` against the workspace's
  enrolled principals. Workspace invitations resolve it with one host-only account point lookup on the broker node,
  and the joiner is admitted only when the host approves a code the joiner's own desktop computed from its device key.
  OS accounts are never enumerated. An invitation binds the resolved UID, canonical username and principal
  generation (workspace) or principal UUID (team or channel)."
- **Protocol contract:** the new methods and projections, `hello` v2, the invitation format, the device code, and the
  statement that broker responses other than `hello` are unauthenticated (SR3).
- **CLI guide:** names select and IDs are still accepted (reverse `cli-guide.md:86`); rewrite the examples with `@bob`,
  `analysis-lab` and quoted `'#methods'`; one "Scripting with IDs" subsection; the invitation join replaces the
  enrollment paragraph; the descriptor table stays for scripted saves.
- **Handoff:** correct any standing instruction that repeats the old §16.4 wording.

## Deferred follow-ups

| # | Follow-up | Why not now |
|---|---|---|
| FU1 | Additional devices approved by the person's existing device (`plan:239` option D), using the same code UI | Needs a signed cross-device approval flow |
| FU2 | Invitation decline, cancel and pruning; "invite to workspace and team in one step" | UI and broker scope; the host already gets **Add {first} to {team}** after admitting |
| FU3 | One-click host setup: the desktop runs `start --name` over the saved SSH connection so the bootstrap key never appears on screen | A new remote-execution route needs its own security review |
| FU4 | Carrying memberships across an account rename (the host confirms `@robert` is the renamed `@bob` on the same UID) | Rare; explicit host decision flow needed |
| FU5 | Team authority transfer and host rename of orphaned teams | Governance change |
| FU6 | A model-facing `channel: "#name"` parameter resolved only within the grant's source channels; friendlier activity text; a Crew tool-call summarizer | Must resolve inside the worker grant, never through `/crew/resolve` |
| FU7 | Authenticated broker responses: an ephemeral X25519 key signed by `W` at `hello`, and a session MAC over `(request id, method, result)` (SR3) | A protocol change with its own review; `expected_username` bounds the risk meanwhile |
| FU8 | Per-device revoke from the person's own device list | Pairs with FU1 |
| FU9 | Allowlist-only remote work folders under a Crew-created prefix (SR4's preferred fix) | Would break saved folders; D15's protected list closes the known path now |
| FU10 | S4: discovery and the two-sided 8-digit code | See [Discovery and short codes](#discovery-and-short-codes-s4-deferred) |

## Related documentation

- [UI redesign specification](ui-redesign-spec.md) — the screens, copy and flows that display these names and run this join
- [Implementation plan](implementation-plan.md) — §5 descriptor, §6 device authority (`plan:237-241`) and §16 naming requirements
- [Protocol contract](protocol-contract.md) — the transport, identity and enrollment contract this design amends
- [Native CLI guide](cli-guide.md) — the examples to rewrite name-first, and the per-account binary prerequisite
- [Implementation status](implementation-status.md) — invariants I01–I03, I12, I16 and I20–I22, and progress on these slices
- [SSH hop policy](ssh-hop-policy.md) — host-key trust rules that the invitation keeps unchanged
