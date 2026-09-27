# Native Crew CLI parity: source inventory and integration plan

Original source/design review on 2026-09-22, before the daemon-owned refactor. The original review performed no product edits, builds, tests or connections. Its inventory, line numbers and proposed command spellings below are historical; use the current implementation map and CLI guide for implemented behavior. Terminal human-authority design is a separate security workstream; nothing below relaxes the existing human-action proof or permits a model to obtain that authority by invoking a CLI.

## Current implemented seams

The original React/Electron orchestration inventory below is superseded by these implemented shared services. This map describes source, not completion of the [acceptance ledger](implementation-status.md). Product artifacts are scoped to `7ab40c81`, broad runtime evidence to `532c3b7d`, and `dd70051e` is a test-only portability checkpoint.

| Current source | Implemented responsibility |
| --- | --- |
| [`commands/crew/args.rs`](../../../crates/biorouter-cli/src/commands/crew/args.rs), [`commands/crew/mod.rs`](../../../crates/biorouter-cli/src/commands/crew/mod.rs), [`daemon_client.rs`](../../../crates/biorouter-cli/src/daemon_client.rs) | Native Crew command tree and typed shared-daemon client; discovery/identity verification precedes explicit human proof. |
| [`daemon_runtime.rs`](../../../crates/biorouter/src/daemon_runtime.rs), [`biorouterd.ts`](../../../ui/desktop/src/biorouterd.ts) | Profile identity, descriptor/lock lifetime and desktop attachment; closing a client does not substitute for an authorized daemon stop. |
| [`crew/authentication.rs`](../../../crates/biorouter/src/crew/authentication.rs), [`crew_authentication.rs`](../../../crates/biorouter-server/src/routes/crew_authentication.rs) | Daemon-owned SSH/PTY sessions and typed input/resize/cancel transport; human prompts stay outside model context. |
| [`crew/transfers.rs`](../../../crates/biorouter-server/src/crew/transfers.rs), [`crew/local_files.rs`](../../../crates/biorouter-server/src/crew/local_files.rs), [`crew_transfers.rs`](../../../crates/biorouter-server/src/routes/crew_transfers.rs) | Daemon transfer orchestration, durable receipts and narrowly approved local file authority; GUI/CLI supply selections and presentation. |
| [`crew/observation.rs`](../../../crates/biorouter/src/crew/observation.rs), [`crew_observation.rs`](../../../crates/biorouter-server/src/routes/crew_observation.rs), [`CrewView.tsx`](../../../ui/desktop/src/components/crew/CrewView.tsx) | Typed daemon room observation replaces independent automatic room-poll loops; clients consume shared identity/state/messages/recovery outcomes. |
| [`shared_conversation.rs`](../../../crates/biorouter-cli/src/commands/shared_conversation.rs), [`cli.rs`](../../../crates/biorouter-cli/src/cli.rs) | Explicit `run`/`session --shared-daemon` uses daemon agent/reply/session services and native continuation recovery. Default local mode remains separate; a grant does not confer human authority on a local worker. |

See the [current CLI guide](cli-guide.md) for actual syntax, and [plan §15](implementation-plan.md#15-daemon-owned-workflows-and-cligui-parity) for unchanged security/completion requirements. Shared IPC remains Unix-only; mixed-GUI and native Windows acceptance are not inferred from these source pointers.

## Historical design and original inventory

Everything below records the pre-refactor inventory and proposed integration. Statements about what “currently” existed, absent APIs, React transfer orchestration, Electron PTY ownership or fixed-port discovery refer to that original checkpoint, not the current implementation. Requirements remain binding unless explicitly superseded in plan §15; the proposed command tree is not a current help reference.

## Architecture decision

Implement `biorouter crew` as a typed client of the same profile-scoped daemon used by the desktop. The daemon owns device credentials, SSH policy/transport, workspace identity, privacy checks, transfers, and agent runs. The remote `biorouter-crew` binary remains the Linux broker/bridge deployment component. Do not construct `CrewManager`, an agent, or a provider inside a Crew CLI command.

The existing code already establishes most of this boundary:

| Existing source | Reuse and implication |
| --- | --- |
| `crates/biorouter-cli/src/cli.rs:48,1437,1843,2755,2788` | Clap tree, command name, setup, and exhaustive dispatcher. Add one `Crew` variant and a dedicated handler in `commands/crew.rs`; also register it in `commands/mod.rs`. Crew requests should not start the standalone tool bridge or enter `build_session`. |
| `crates/biorouter-cli/src/commands/session_watch.rs:12-23,60,88,810,1179` | Existing precedent for thin terminal clients: session attach does not create a second Agent. Extract its authenticated JSON/SSE transport rather than copy it. Current client reads daemon secret from an environment variable and uses fixed loopback/configured port; that is not new Crew discovery. |
| `crates/biorouter/src/crew/mod.rs:120-137,429,447,660,724,847,988,1057,1155,1167,1272` | One manager holds saved connections and live transports. Reuse prepare/save/authenticate/connect/sign/request, provider checks, run admission, scoped tool dispatch and revocation. Its registry/mutex map is process-local; creating another manager in every CLI process would split ownership. |
| `crates/biorouter-server/src/routes/crew.rs:224-379,103-130,380,1059,1193` | Existing connection, enrollment-device, typed task, cancellation and session-grant API. All routes require human proof, including reads. Task admission/ledger/idempotency remain in the daemon; its ledger already refuses a second writer. |
| `crates/biorouter-crew/src/lib.rs:12-159`; `src/broker.rs:1097,1222,1383` | Existing remote protocol and authoritative membership/privacy/history/blob operations. Keep business rules here and in the manager, not in terminal argument handlers. |
| `ui/desktop/src/components/crew/crewApi.ts:101-139` | Desktop currently sends string method/JSON requests and manually declares response interfaces. Replace hand-maintained client shapes with generated types from the shared API contract. |
| `ui/desktop/src/components/crew/CrewFiles.tsx:79-200`; `crewTransfers.ts:1-113` | Upload hash/resume/chunk/commit orchestration and pending state currently live in React/localStorage. Move that orchestration and pending transfer metadata into a daemon service so CLI and GUI cannot diverge. Clients still select/open local files and stream their bytes. |
| `ui/desktop/src/components/crew/CrewView.tsx:129-175,225-252` | Room UI currently polls snapshot/history/task state every four seconds. There is no existing Crew room subscription endpoint to claim as reusable. |
| `ui/desktop/src/main.ts:4674-4745`; `components/crew/CrewAuthentication.tsx:78-123` | SSH auth plan is daemon-generated, but Electron owns the PTY/master lifetime. Extract a shared authentication-session coordinator; terminal/renderer only provide input, output and resize events. Credential prompts must remain outside chat/history/model context. |
| `crates/biorouter-crew/src/main.rs:3-49` | Remote binary offers broker `serve/start/status/stop` and `bridge`, not human chat commands. Do not confuse it with native `biorouter crew`. |

## Concrete command tree

All workspace operations take an explicit `--connection ID` or an unambiguous selected connection. Channel operations likewise resolve `--channel ID`; human-readable names may be accepted only when unique in the specified workspace/team. The daemon checks every identifier and permission again. Selection is convenience, not authority.

```text
biorouter crew
  status
  connections list|show|add|update|remove
  auth CONNECTION
  connect CONNECTION
  disconnect CONNECTION
  workspace show|bootstrap
  enroll prepare|invite|accept|revoke
  members list
  teams list|create
  channels list|create|archive|mark-read
  invites list|create|accept
  ownership offer|accept
  membership revoke
  profile show|set
  history [--before CURSOR|--after CURSOR] [--limit N]
  search QUERY
  watch [--after CURSOR] [--output-format text|stream-json]
  send [--text TEXT|--input FILE|-] [--attachment BLOB_ID] [--reference ID]
  files upload|resume|status|pending|download|reference|show-reference
  tasks start|list|show|watch|cancel
  grants list|grant|revoke
  privacy show|set-personal|set-workspace
```

Provide `--output-format text|json|stream-json`, stable error codes/exit statuses, a deliberate request ID for retryable mutations, and `--no-start` for callers that require an existing daemon. Machine-readable stdout contains only results/events; prompts/progress go to stderr or the controlling terminal. Avoid storing message bodies, tokens or private paths in a global CLI command history. `watch` cancellation detaches observation; task cancellation is the separate explicit command.

Mapping to the existing services:

- Connections use current `/crew/connections` routes. `auth` uses the common authentication coordinator and `connect` then performs the existing cryptographic workspace/node verification. `workspace bootstrap` maps to `auth.bootstrap`; enrollment preparation uses `/crew/devices/prepare`, invitation/accept/revoke use `enrollment.invite`, `auth.enroll`, `enrollment.revoke`. Accept the enrollment token through secure input rather than requiring argv.
- Workspace, members, teams, channels, profile and invitations read the ACL-filtered `workspace.snapshot`. Mutations map to `team.create`, `channel.create`, `channel.archive`, `channel.read`, `profile.update`, `invitation.create`, `invitation.accept`, `channel.transfer`, `transfer.accept`, `membership.revoke`. Transfers preserve current-owner semantics; the broker currently removes the previous owner from channel membership on acceptance (`broker.rs:1705-1721`), which both clients must state accurately.
- History/search/send map to `messages.history`, `messages.search`, `message.post`. Cursor values are opaque strings, not client-generated sequence numbers. Expose body input from stdin/files without shell interpolation; keep the same request ID across an ambiguous retry.
- Files reuse `blob.begin/status/chunk/finish/read` and `reference.create/get`. Upload/commit is separate from publishing an attachment in `message.post`. Download to an explicitly selected output, with bounded streaming, integrity verification and atomic completion; do not trust an attachment name as a local filesystem path. Current desktop transfer cap is 64 MiB, broker per-blob cap is 1 GiB (`CrewFiles.tsx:115`, `broker.rs:2007`); define a shared advertised limit rather than silently presenting those as parity.
- Tasks use existing typed `/runs` start/list/cancel routes; `tasks watch` follows the returned daemon `session_id` through the current session observer API. Never implement task start as generic `run.create` or local `biorouter run`. Preserve provider resolution, posting consent, context-channel selection, idempotency, capacity and unknown-outcome handling already in `routes/crew.rs:380-550`.
- Grants use the existing typed session-grant service, with new sanitized list/revoke routes as needed. The current task list is a local task ledger, not a complete grant inventory: a grant added to an existing conversation need not be a task created by `/runs`. Generic HTTP requests deliberately forbid `run.*`/`worker.*` (`routes/crew.rs:328`); do not bypass that gate to fill the CLI gap.
- Privacy read displays personal connection mode, authoritative workspace policy/epoch, channel classification and effective restrictions. Personal mode uses the existing connection policy update; workspace mode uses `policy.set`. Do not introduce a client-only toggle or a generic “make this data public” command. The existing origin/provider/channel restrictions remain authoritative.

Not all conceivable chat actions exist today: there is no invitation decline/revoke, team deletion, channel rename, team ownership transfer, or dedicated DM method in the current broker dispatch. Do not advertise these as CLI/GUI parity until implemented once in the shared service.

## Typed service/client seam

Add daemon API DTOs under `crates/biorouter/src/crew/api.rs`: typed request/response/error/cursor/event structures, deriving serde and schema traits. Reuse the existing connection/mode types. Keep internal credentials, signing keys and worker bearer tokens out of public DTOs. Move `StartRunRequest`, `RunView` and `GrantSessionRequest` from route-local definitions into that contract. Represent allowed human operations with an explicit tagged enum and per-operation structs; normalize once into the existing signed broker request. Do not publish arbitrary method/JSON passthrough as the user-facing command interface.

Keep `CrewManager` as the SSH/policy service. Factor task orchestration currently embedded in `routes/crew.rs` into a daemon-owned `CrewTaskService` that receives `AppState`; routes only authenticate, deserialize and call it. Add daemon-owned transfer and room-observer services where orchestration currently exists only in the GUI. This is shared process logic, not a second remote backend or database requirement.

Extract a reusable Rust `DaemonClient` from the current session client, then expose typed methods through `CrewClient`. It carries endpoint identity, ordinary daemon authentication, caller scope and the separately designed human/worker authority. It must distinguish transport failure, confirmed refusal and unknown mutation outcome. Do not automatically retry a mutation with a new idempotency key. Reuse existing socket non-inheritance and environment scrubbing. Existing CLI JSON transport reads the whole response, and its SSE parser has a narrow current framing assumption; put explicit size bounds and typed stream framing in the extracted client before using it for files/general Crew events.

Generate desktop OpenAPI types/SDK through the repository generator and migrate `crewApi.ts` to them. GUI and Rust clients then share the same DTO schema and daemon behaviors. Neither copies broker authorization, provider-tier decisions, transfer retry policy, or task admission.

For room `watch`, initially let the daemon coordinate bounded polling of the existing cursor-based history and snapshots, emitting a typed NDJSON/SSE stream to both clients. Do not duplicate separate polling state machines in React and the CLI. Revalidate access on each poll/reconnect, propagate revocation rather than continue serving stale cached data, drain all pages, preserve cursors, and specify recovery from `stale_cursor`. Session task SSE is reusable for task activity; it does not substitute for a room stream or promise durable room replay.

## Electron-closed daemon lifecycle

Existing `apps.rs:226,307` discovers only a fixed loopback/configured port and unauthenticated `/status` success. `/status` returns plain `ok` (`biorouter-server/src/routes/status.rs:16`). Electron intentionally owns a random private port/secret and kills its daemon on app exit (`ui/desktop/src/biorouterd.ts:314,523-545`). `biorouterdSingleton.ts:46` is only an in-process JavaScript singleton. These mechanisms cannot be described as a shared terminal/GUI lifecycle without changes.

Create one profile-scoped Rust lifecycle implementation, used by both clients. Prefer a global `biorouter daemon start|status|stop` interface, since this is the common daemon, not a new Crew-only process. Ordinary Crew commands may ensure it is running; they must not shut down a reused instance when exiting.

1. Resolve the same absolute canonical profile identity for both launchers; `Paths` currently accepts relative overrides (`crates/biorouter/src/config/paths.rs:7`). Serialize startup with an early cross-process lock, before mutable services load.
2. Publish an atomic private discovery descriptor containing endpoint, instance/profile identity, PID/start identity and protocol/build compatibility. Authenticate the discovered daemon before reuse; neither PID existence nor an HTTP 200 is sufficient. Credential handoff and terminal proof follow the separate authority design, not a plaintext broadly readable token or inherited model environment.
3. Reuse binary resolution in `commands/exe_path.rs:80`, including installed Windows breadcrumbs and matching packaged binaries. Launch API-only on explicit loopback with intentional profile/environment, without web assets or a browser. `commands/serve.rs:127,342` has useful stop/reap primitives, but its web-bundle dependency and child-ends-with-command behavior must remain separate.
4. Daemon and authentication-session lifetimes outlive attached clients. GUI quit or terminal exit detaches. Explicit daemon stop uses an authenticated portable drain/shutdown operation, reports active tasks, revokes/cleans up as specified and waits boundedly; it never kills a different profile's process or an unrelated external daemon. A native Windows shutdown path cannot assume Unix signals.
5. On restart, restore metadata and mark interrupted runs/SSH transports honestly; do not silently rerun agent prompts or replay unknown mutations. `CrewManager::new` already resets connection statuses to disconnected (`crew/mod.rs:299`). Preserve server ledger unknown-outcome handling and the single-writer lock.

Acceptance must ultimately cover CLI-only startup/authentication, GUI attachment to the same daemon, GUI closure during a CLI-owned task, concurrent clients and idempotency, absent desktop assets, stale discovery/profile mismatch, grants/ownership/privacy refusal parity, cursor reconnection, resumed transfers, daemon restart, and separately qualified Windows behavior. This document is source evidence and design only; none of those execution claims was established here.
