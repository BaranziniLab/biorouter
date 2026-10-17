# Open workspaces and automatic setup

> **What this is.** The design contract for six Crew changes made together in October 2026: automatic server setup, open (shared) workspaces, deletion, `@agent` from the composer, a default model for Crew tasks, and agents that run their work on the server with a per-workspace context document.
> **Status:** Current. Implemented on the `claude/crew-auto-setup` branch; where this page and the code disagree, the code is what shipped.
> **Audience:** Developers and reviewers of Crew. Users read the [Crew user manual](../README.md).

## Why

A lab that tried Crew on `leo.ucsf.edu` (Ubuntu 22.04, Linux 5.15, no job scheduler) hit six walls:

1. **No setup.** Nothing installed `~/.local/bin/biorouter-crew`; the app showed copy-paste commands with a placeholder path, and "Start it for me" could not sign in to a password-only account.
2. **Members could not use agents with files.** Every agent folder had to be owned by the member, inside a confined sandbox that requires Landlock ABI 3 (Linux 6.2+). On leo every command failed for everyone, and no channel file could reach the server.
3. **Owner-only management.** Only owners renamed, archived or added people, and every addition ended every agent grant in the workspace.
4. **No deletion** of channels, teams or workspaces.
5. **One way to start an agent:** the **Ask my agent** button, with the chat's default model only.
6. **Agents knew nothing about the server** they were meant to compute on.

The owner rulings that shape the answers: *user warns, agent never*; *privacy is safety, not security*; *local agent forgery is not a threat*; keep the user experience simple; keep model-tier refusals.

## Decisions

| ID | Decision |
|---|---|
| OW-1 | A workspace has a **governance** setting, `open` or `managed`. Journals written before this change read as `managed` (today's rules). A workspace created by this release's host flow is set to `open` right after bootstrap. Only the host changes it. |
| OW-2 | In an **open** workspace every active member may rename, archive and delete teams and channels, add and remove people in teams and channels, join any team or channel, and invite and approve joiners. Only the host removes someone from the workspace, changes privacy or governance, renames or deletes the workspace. |
| OW-3 | In an open workspace a new member is added to every team and every unarchived channel when they are admitted; a new team gets every active member; a new channel gets every member of its team. Additions in an open workspace do **not** bump the policy epoch, so they no longer end agent grants. Removals, archive, delete and policy changes still do. |
| OW-4 | `channel.delete`, `team.delete` and `workspace.delete` exist. Channel and team deletion remove live state (the channel or team, its attachments' state entries and files, references, invitations and read positions) and revoke runs that name the channel. Message records stay in the journal but become unreadable to everyone, because their channel no longer exists. They do not bump the epoch. A team's `#general` is deleted only with its team. |
| OW-5 | `workspace.delete` is host-only. It commits a tombstone (devices, grants, joins cleared, every run revoked), answers every later request except `hello` with `workspace_deleted` (and `hello` with `"state": "deleted"`), waits 30 seconds so connected members learn of it, then erases only the files the broker created, leaves a runtime tombstone (below) and exits. `biorouter-crew delete` does the same for a stopped broker. A member's daemon that sends nothing in those 30 seconds (its heartbeat waits two idle minutes), or is offline, learns of it from the runtime tombstone (L7). |
| OW-6 | A connection's agent access has three levels: **off** (no work folder), **folder** (today's confined mode) and **full** (*run as me*). Full runs commands as the member's own account through their own bridge: login shell, network, child processes, any path the account can reach, jobs that may outlive the task. The member chooses it for their own account; new connections made by the setup flow default to full with one warning. Public models never get any level (all four existing layers still refuse them). |
| OW-7 | In full mode a channel attachment can be copied to the server (`remote.fetch_blob`) and a server file can be posted to the channel (`remote.share_file`), up to 1 GiB, through a background transfer in the bridge. |
| OW-8 | Each workspace has a **context document** (Markdown, at most 64 KiB) stored by the broker, with a revision, editor and time. The host flow generates it from a fixed server probe right after bootstrap. Any member edits it in an open workspace; only the host in a managed one. Every member's Crew agent receives it as reference information, framed as authorizing nothing. |
| OW-9 | **Setup is a daemon service** used by the GUI and the CLI: probe, sign in if needed, transfer the broker over the SSH session's stdin, verify its SHA-256, install it atomically at `~/.local/bin/biorouter-crew`, then start the workspace (host) or connect (member). The broker comes from the desktop build itself (`crew-broker/linux-x86_64/` next to `biorouterd`, with a manifest); a development override is `BIOROUTER_CREW_BROKER_DIR`. |
| OW-10 | `@agent`, `/agent` and `/task` at the start of a Crew message start a task in that channel with the person's model for Crew tasks. The composer shows the same facts the pane shows before Enter; Enter is the consent. A person whose username is `agent` keeps `@agent` as their mention; `/agent` still works. |
| OW-11 | The model for Crew tasks is the config key `CREW_TASK_MODEL` = `{provider, model}`. The renderer and the CLI resolve it (falling back to `BIOROUTER_PROVIDER`/`BIOROUTER_MODEL`) and send it explicitly; the daemon never reads it, so it is a preference, not a capability key. Every start is still admitted by the daemon and the broker. |
| OW-12 | Crew task limits rise to 60 model turns, 200 tool calls and 55 minutes, because full-access tasks do real work. |

## Broker wire contract (server, `crates/biorouter-crew`)

All additions are compatible with a 1.92.1 broker reading the same journal: new state fields use `serde(default, skip_serializing_if)`, deletions are map `Remove` patches, and new behavior is a new method or capability. Capabilities are appended to `capabilities()` after `presence_v1`, in this order: `open_governance_v1`, `delete_v1`, `workspace_context_v1`, `remote_access_v1`, `wide_runs_v1`.

### State

- `Workspace.governance`: `"managed"` (default, omitted) or `"open"`.
- `Workspace.deleted_at`: unix seconds, omitted until deleted.
- `State.context` (or `Workspace.context`): `{text, revision, updated_by, updated_at}`, omitted until first set.
- `Run.remote_access`: `"folder"` (default, omitted) or `"full"`.

### Methods

| Method | Who | Params | Result |
|---|---|---|---|
| `workspace.set_governance` | host, person | `{governance: "open"\|"managed"}` | `{governance}` |
| `team.join` | open: any active member | `{team_id}` | `{team_id, joined: true}`; also joins its `#general` |
| `channel.join` | open: any member of the channel's team | `{channel_id}` | `{channel_id, joined: true}` |
| `channel.delete` | managed: owner, or host stewarding an orphan; open: any channel member; always a person | `{channel_id, expected_name?}` | `{deleted: true, channel_id, team_id, removed_attachments, removed_references}` |
| `team.delete` | managed: creator, or host when the creator left; open: any team member; person | `{team_id, expected_name?}` | `{deleted: true, team_id, channels}` |
| `workspace.delete` | host, person | `{workspace_id, expected_name}` | `{deleted: true, workspace_id, erase_after_seconds: 30}` |
| `workspace.context.set` | open: any member; managed: host; person | `{text, expected_revision}` | `{revision, updated_at}` |
| `workspace.context.get` | any member; a worker of a private run, or of a public run when the workspace is Public | `{}` | `{context: {...} \| null}` |

Refusals keep the broker's `code: sentence` form. New codes: `workspace_deleted`, `revision_conflict`, `name_changed`. `#general` alone: `invalid_params: #general belongs to its team. Delete the team instead.`

**Runtime tombstone (OW-5, L7).** Once the erase is done, and only while the runtime socket is still the one the broker bound, the broker writes `deleted` (the workspace ID, mode 0644, created with `O_EXCL | O_NOFOLLOW`) into the runtime directory, then removes the socket and keeps the directory. If the file cannot be written the directory goes with the socket, as before. It is written even when the erase failed, because the host deleted the workspace either way. `biorouter-crew delete` (and `serve` finishing an interrupted erase) retires the recorded runtime directory the same way: a stale socket goes, an existing tombstone is kept, and a directory with no ID to name is removed. A member's bridge reads it only when no broker answers for the workspace at the saved path or in a directory it moved to: the directory must be the owner's (not a symbolic link, not group or other writable), the file a regular file the owner owns with one link, opened with `O_NOFOLLOW | O_NONBLOCK` and read up to 128 bytes, naming exactly the workspace asked for. Then the bridge fails with `workspace_deleted: The host deleted this workspace. Remove it from this computer.` and exits 1, and so does a connected bridge whose broker went and left the tombstone in its own directory. `start` never reuses or removes a tombstone: a new workspace, even in the same state directory, gets a new random runtime directory, and a recorded directory that holds one (only a deleted workspace restored from a backup) reads as unusable, so the workspace moves. The tombstone lasts until `/tmp` is cleaned; a member offline past that reads the workspace as not running.

`workspace.snapshot` adds `workspace.governance`, `context` (text, revision, updated_by, updated_at) and, in an open workspace, `joinable_teams` and `joinable_channels` (`{id, team_id?, name, display_name, handle, classification, member_count}`), inside the existing snapshot byte budget.

`run.create` accepts `remote_access: "full"` only with `remote_root`, `remote_execution: true` and a private provider; it echoes `remote_access` in the run. With `wide_runs_v1` a run may read up to 100 source channels (was 20). `run.remote_scope` adds `access` and `owner_id`.

No new method is in the worker allowlist except `workspace.context.get`. Every mutation above also checks `actor.run.is_none()`.

### Bridge-local methods (member's own binary)

The bridge answers these itself and never forwards them:

- `bridge.hello {}` → `{bridge_version, capabilities: ["remote_full_v1", "remote_transfer_v1", "bridge_probe_v1"], landlock_abi, confined_execution_supported, user, uid, home, shell}`. An old bridge forwards it and the broker refuses, which the daemon reads as "old bridge".
- `bridge.probe {}` → the server facts below.

Full-access `remote.*` methods, used only when `run.remote_scope` answers `access: "full"` (a missing `access` means folder mode, so an old broker fails closed):

| Method | Params | Notes |
|---|---|---|
| `remote.shell` | `{command, cwd?, timeout_seconds?, wait_seconds?, detach?, idempotency_key}` | Runs `<login shell> -lc <command>` as the member, stdin `/dev/null`, output to `~/.local/state/biorouter-crew/jobs/<id>/`. Waits up to `wait_seconds` (default 10, at most 20) and returns the result or `running` with the job id. `detach: true` survives the bridge. |
| `remote.execute` | `{argv, ...same}` | As `remote.shell` with an argv. |
| `remote.job_status` | `{job_id, tail_bytes?}` | Any run of the same owner may read it. |
| `remote.cancel` | `{job_id}` | Kills the job's process group. |
| `remote.list`, `remote.stat`, `remote.read`, `remote.write`, `remote.hash` | absolute, `~/` or work-folder-relative `path`; `read {offset, length ≤ 393216}`; `write {text \| data_hex, mode: truncate\|append\|create_new, create_parents}` | Symlinks followed, no ownership rule. |
| `remote.fetch_blob` | `{blob_id, path, idempotency_key}` | Background copy of a channel attachment, SHA-256 checked; returns a job id. |
| `remote.share_file` | `{path, name?, media_type?, idempotency_key}` | Background upload of a server file (≤ 1 GiB); the job result carries the `blob_id` the daemon then posts. |

Folder mode is unchanged, except that a kernel without Landlock ABI 3 is refused up front with a sentence that names full access.

### Server probe and context draft

`biorouter-crew probe --output-format json` and `bridge.probe` run fixed, bounded checks (no network, 5 s each) and return:

```json
{"schema": 1, "host": "leo.ucsf.edu", "os": "Ubuntu 22.04.5 LTS", "kernel": "5.15.0-141-generic",
 "arch": "x86_64", "glibc": "2.35", "cpus": 16, "mem_gib": 187, "gpus": [],
 "scheduler": {"kind": null, "commands": []}, "modules": false,
 "conda": ["~/anaconda3"], "containers": {"docker": true, "docker_group": true, "apptainer": false, "podman": false},
 "languages": {"python3": "3.x", "R": "4.x"},
 "storage": [{"path": "/pool1/home", "fs": "zfs", "size_gib": 20000, "avail_gib": 19900}],
 "landlock_abi": 1, "user": "wanjun", "home": "/pool1/home/wanjun"}
```

It never reads environment variables, process lists, other people's homes, SSH configuration or history files. `biorouter-crew doctor --output-format json` adds hosting blockers and execution limits. The daemon renders the facts as the first context document (`crew::context::render_workspace_context`).

## Desktop daemon contract (`crates/biorouter`, `crates/biorouter-server`)

### Setup routes (all require proof of a person)

| Route | Body → answer |
|---|---|
| `GET /crew/setup/artifact` | → `{version, target, sha256, size, source: "bundled"\|"dev"}` |
| `POST /crew/setup` | `{target: {kind: "connection", connection_id} \| {kind: "pending", preparation_id, ssh_target, port?, identity_file?, proxy_jump?}, intent: "check"\|"install"\|"host"\|"member", workspace_name?, replace_existing?, agent_access?: "off"\|"folder"\|"full", fix_storage_permissions?}` → `SetupStatus` |
| `GET /crew/setup/{job_id}` | → `SetupStatus {job_id, intent, state: running\|needs_sign_in\|finished\|failed\|cancelled, stage, stages[], server, installed, artifact, outcome, error}` |
| `DELETE /crew/setup/{job_id}` | cancel |
| `POST /crew/setup/{job_id}/authentication` | `{request_id, controller_id, cols, rows}` → an authentication session; the terminal is the existing `GET /crew/authentication/{id}/terminal` |
| `POST /crew/connections/{id}/context/draft` | → `{draft, facts}` from `bridge.probe` over that connection |

`SetupStatus.outcome` carries `installed_version`, `reused`, `upgraded_from` (the version and digest it replaced, or null), `host` (the same start output as `/crew/host/start`), `host_command`, `connection_id`, `work_folder` (the canonical `~/crew-work/<workspace>` the job created), `remote_access` and `context_draft`. A member job changes the connection's agent access only when the request carries `agent_access`: `full` sets the work folder (when there is none), commands and full access where the broker and bridge support it, `folder` keeps an existing folder as it is, and `off` changes nothing. The desktop Join sends the level the person chose; the CLI sends `--agent-access`, defaulting to `full` only for a connection with no work folder. When the installed broker reports this version with a different SHA-256 it is replaced atomically and the old copy kept as `.previous`; a newer version is never replaced. A host job whose `$HOME/.local/share/biorouter-crew` has a group- or other-writable ancestor without the sticky bit stops before start with `crew_host_storage_unsafe`, naming the folders; with `fix_storage_permissions` it runs `chmod go-w` on exactly those folders (owned by the account, inside `$HOME`) and continues. Any start failure carries the broker's own output in `error.detail`. A setup sign-in rides the same control path the connection will use, so one password covers install, start, create and connect. A sign-in whose bridge is missing keeps its SSH master so setup can use it.

### Connection, runs and agents

- `Connection.remote_access`: `"folder"` (default) or `"full"`; also on `SaveConnection` and the invitation's advanced fields. Full requires a work folder and `remote_execution`.
- The connection view adds `server_access: {full_access_available, confined_execution_available, bridge_version, landlock_abi, user, home}` from `bridge.hello`, or `null` before the first connect.
- `run.create` sends `remote_access` only when the broker advertises `remote_access_v1` and the bridge `remote_full_v1`, and checks the echo.
- `crew__request` gains `remote.shell`, `remote.stat`, `remote.fetch_blob`, `remote.share_file`, `blob.list` (when advertised) and `workspace.context.get`. The tool instructions say what full mode can do, that heavy work runs on the server, and to read the workspace context.
- Every Crew task and connected chat gets the workspace context inside the escaped hidden context, framed as reference information that authorizes nothing.
- `POST /crew/connections/{id}/workspace/delete {workspace_id, expected_name}` sends `workspace.delete` and then removes the connection from this computer. A `workspace_deleted` refusal marks a member's connection so the interface offers **Remove from this computer**. So do a verified `hello` whose unsigned `state` is `deleted`, and a bridge that exits with the `workspace_deleted` line (`SshFailureKind::WorkspaceDeleted`, code `crew_workspace_deleted`), at a person's Connect, a keepalive re-dial or a request alike (L7). A marked connection is never dialled again, a person's Connect included: the retries are disarmed, every request and Connect is answered `409 crew_workspace_deleted`, the connection view reports `workspace_deleted: true`, and a connection that a keepalive re-dial or heartbeat left disconnected has `last_error_code` `crew_workspace_deleted`. The mark is memory only, so after a daemon restart the next connect finds it again.

## Interfaces

### Desktop

- **Host dialog:** "Set up and start Crew" runs a host setup job, shows plain stages (Checking server, Signing in, Installing Crew in your account, Starting workspace, Ready), then Create bootstraps, sets the institution, sets governance to open and saves the context draft.
- **Not set up / Join:** "Set up Crew for my account" runs a member setup job and reconnects; Join starts it by itself when the bridge is missing.
- **Composer:** `@agent`, `/agent`, `/task` with a suggestion list (agent row and people), an inline consent line, and the send button labelled "Start agent task".
- **Settings > Models:** a "Crew tasks" row ("Same as new chats" or a model); the task pane starts on it.
- **Workspace settings:** governance switch (host), a "Context" tab with an editor and **Draft from server**, and **Delete workspace…** (host, type the name). Members get **Remove from this computer**.
- **Sidebar and channel:** **Delete channel…** and **Delete team…** wherever the person may; **Browse channels** lists joinable teams and channels in an open workspace.
- **Connection settings:** "What can your agent do on {server}?" Off / In a folder / As me on the server.

### CLI

| Command | Does |
|---|---|
| `biorouter crew setup --server LOGIN` / `--connection NAME` [`--replace-existing`] | install or update the broker in your account |
| `biorouter crew doctor --server LOGIN` | probe only |
| `biorouter crew host --name NAME --server LOGIN [--institution ID] [--mode private\|public]` | prepare, install, start, save, sign in, bootstrap, privacy, open governance, context |
| `biorouter crew channels delete CHANNEL [--yes]`, `channels join CHANNEL` | |
| `biorouter crew teams delete TEAM [--yes]`, `teams join TEAM` | |
| `biorouter crew workspace delete --confirm NAME` | host |
| `biorouter crew workspace governance open\|managed` | host |
| `biorouter crew workspace context show\|set FILE\|edit\|draft` | |
| `biorouter crew connections set-access off\|folder\|full [--folder PATH]` | |
| `biorouter crew tasks start CHANNEL --text T` | `--provider`/`--model` optional (resolved from `CREW_TASK_MODEL`) |
| `biorouter crew ask CHANNEL TEXT…` | `tasks start` with the defaults and posting allowed |
| `biorouter crew defaults show\|set --provider P --model M\|clear` | the model for Crew tasks |

## Risks accepted

- **Full access is the member's whole account.** A misled agent can run anything the account can, including replacing its own bridge (attacker M2 in the [naming design](naming-design.md)). Device keys stay on the desktop, so it still cannot sign as the person. The connection warning says so, and every `remote.shell` is announced in the channel by program name and length, never the command text.
- **Prompt injection.** Other members' messages, files and the context document reach every member's agent. The agent's instructions treat them as information. Model-tier refusals, the secret guard on arguments and the secret-output scan on results still apply.
- **Deleted channels' text stays in the journal** until the workspace is deleted. The interface says "removed from the workspace".
- **Open governance trusts every member** with everything but removal from the workspace. It is the host's choice and the host can switch back.

## Related documentation

- [Broker protocol](protocol-contract.md): the wire contract this page extends.
- [Naming design](naming-design.md): identity rules, attacker M2.
- [Linux portability](linux-portability.md): the broker build and its glibc floor.
- [Agents and chat access](../agents-and-chat-access.md): what users see of agents.
- [Privacy tiers](../../security/privacy-tiers.md): the model-tier rules Crew keeps.
