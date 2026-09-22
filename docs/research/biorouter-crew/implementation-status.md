# BioRouter Crew implementation and acceptance status

Status is recorded against the final revision under test. A lower-level fixture
or unit test never upgrades a real-app, cross-platform, or institutional gate.
Each row must have an evidence path before it can be marked `pass`.

Legend: `not-run` = no execution yet; `blocked` = an external prerequisite is
missing; `fail` = an observed defect; `pass` = the required behavior was
observed on the same revision. The implementation checkpoint is `e9a09c6c`
plus subsequent product hardening and Luna regressions in this worktree.
Focused broker (22 cases), managed-policy, MCP and CrewManager tests have
passed. The complete required check sequence and fresh application binaries
remain under validation. Alice's initial host setup and message have app and
journal observations. The previous Bob enrollment fixture was manually
altered outside the normal save flow and is not nominal product evidence;
Carol's completed UI enrollment was not established. A clean three-profile
run is required. See [fixture integrity correction](qa-fixture-integrity.md).

## Release gates

| ID | Gate and acceptance invariant | Type/evidence required | Status | Owner / next action |
|---|---|---|---|---|
| G01 | Actual BioRouter Electron development app is built from this worktree and three independent app clients drive the visible Crew UI. | Build command, revision, sanitized screenshots/video, action timeline | not-run | Alice initial setup/post and Bob visible onboarding were observed, but the previous three-client claim lacked complete Carol evidence and Bob's enrollment fixture was manually altered. Revalidate three independent, simultaneous clients with fresh profile/binary provenance; see `qa-fixture-integrity.md`. |
| G02 | Three real Unix accounts use distinct UIDs, home directories, SSH identities and profiles; no shared key or copied provider credential. | Disposable AWS fixture report with account/key/UID mapping | blocked | AWS fixture ready; source transfer approval pending |
| G03 | Alice hosts the broker from a user-writable home; product install/start/restart/recovery is rootless and creates no system account/group/service/global SSH change. | Privilege trace, filesystem manifest, process UID, install/recovery transcript | not-run | Alice's Linux fixture install/start passed as UID 1101 with state under `/home/alice/.local/share/biorouter-crew/lab`; restart/recovery and no-global-change audit remain. |
| G04 | Alice, Bob and Carol complete connect/enroll/general/analysis/files/owned-agent/context/revocation/recovery workflow with human chat before model selection. | Three-app evidence bundle and event IDs | not-run | Alice alone connected to the local broker, initialized the workspace, created `Synthetic Lab/#general`, and posted a human message visible as Restricted. Broker journal sequences 4 (auth.bootstrap), 5 (team.create), and 6 (message.post) confirm the effects. Bob/Carol enrollment and the multi-user workflow remain pending. |
| G05 | Real provider positive pass uses the actual BioRouter provider path and a public-safe fixture; negative pass uses a designated synthetic public sink and observes zero forbidden canary bytes. | Provider identity, sink digest/bytes, dispatch/tool/context traces | not-run | Parent/provider + QA |
| G06 | 10, 30 and 50 independently authenticated Unix participants exercise bounded load; the 50-user target is a 30-minute soak with three graphical clients retained. | Workload seed, p50/p95/p99, CPU/RSS/open files, journal/replay, UI responsiveness | not-run | QA; run only after G04 |
| G07 | macOS, Windows and Linux client compatibility is measured separately for OpenSSH/MFA/process lifecycle. | Per-OS run records; no macOS substitution for other OSes | not-run | QA/platform |
| G08 | All critical/high ownership, privacy, data-loss and provider-routing defects are fixed and replayed on one final revision. | Independent adversarial review, regression IDs, retest evidence | not-run | Parent + QA |
| G09 | AWS instances, volumes, security groups, keys and fixture credentials are deleted and independently verified. | AWS cleanup report and post-cleanup resource queries | not-run | QA; mandatory after every launch |

## Requirement invariants

| ID | Invariant from implementation plan | Focused check / negative case | Status |
|---|---|---|---|
| I01 | Authenticated SSH discovers a stable workspace UUID; aliases/jump paths reach the same workspace. | Alias identity comparison; wrong workspace descriptor denied | not-run |
| I02 | Broker derives UID from the kernel and binds verified enrolled principal; nickname never grants authority. | Forged username/device, recycled UID generation, same nickname collision | not-run |
| I03 | Team/channel membership, invitations, discoverability and multi-team membership are explicit records. | Invite does not create SSH account; OS account enumeration is absent | not-run |
| I04 | Creator is initial channel owner; only current owner archives/transfers; transfer requires successor acceptance and revokes stale owner capabilities. | Non-owner removal; stale invitation/approval after transfer; immutable creator audit | not-run |
| I05 | Human messages and structured agent events share a room; only the run owner can invoke, steer, approve or cancel. | Mention does not create a run; Bob controls neither Alice nor Carol's run | not-run |
| I06 | Attachments are opaque, durable, resumable, ACL checked and digest verified; unsupported previews remain downloadable. | Revocation between chunks; traversal/symlink/hardlink/special-file/archive active-content cases | not-run |
| I07 | Search/fetch and derived outputs preserve every source ACL and restriction. | Restricted `methods` cannot leak to `analysis` or a later member; policy changes between search/fetch | not-run |
| I08 | Effective admission is the intersection of identity, membership, ACL, shared baseline, personal mode, source restrictions, run policy and endpoint/tool scope. | Any one denial wins; unknown label/endpoint fails closed | not-run |
| I09 | Private cluster/workspace blocks every public model, embedding, OCR, summary, plugin and saved-SSH path. | UI, slash, natural-language MCP, reconnect, alias and scheduled-job paths; zero canary bytes at sink | not-run |
| I10 | Public mode permits only public-safe inputs/tools; Private→Public never relabels existing history/files/caches. | Restricted object ID, retained private history, fresh public-safe run | not-run |
| I11 | Policy epoch changes invalidate incompatible grants; Public→Private blocks queued/new work and reports already-submitted work honestly. | Toggle during queue/stream; stale worker grant rejected | not-run |
| I12 | Workers run as invoking user's UID/job allocation; unrestricted same-UID host trust boundary is documented and arbitrary tools are withheld when isolation is unavailable. | Process/job owner; no sudo/container/new account/group; unsupported feature has actionable refusal | not-run |
| I13 | One canonical ordered journal is the source of truth; records are bounded, checksummed, idempotent and single-writer. | Duplicate key same/different payload, concurrent writer, lock/fencing, oversized frame | not-run |
| I14 | Acknowledge only after flush+fsync; crash/replay verifies continuity/checksums/invariants and only quarantines an incomplete final record. | Torn tail vs interior corruption; no skipped revocation; snapshot atomicity | not-run |
| I15 | Home/NFS capability is measured for locking, fsync, atomic rename, quota and disconnect; unsupported storage suspends writes. | Controlled disk-full/I/O failure; no silent `/tmp` canonical store or competing writer | not-run |
| I16 | Versioned UTF-8 JSONL framing enforces limits, protocol negotiation, opaque cursors, bounded queues and retry horizon. | Invalid version/UTF-8/frame, slow consumer resync, cursor room isolation, expired idempotency key | not-run |
| I17 | Attachment chunks use constant memory; commit fsyncs bytes, renames atomically, fsyncs directory, then journals publication. | Failure before/after each flush/fsync/rename/ack boundary; orphan grace sweep | not-run |
| I18 | SSH state machine and per-hop trust/MFA are explicit; changed keys, route denial and reauth are distinguishable. | Keyboard-interactive multi-prompt, TOTP/Duo, passphrase/security key, expired auth, changed key, forced SFTP-only | not-run |
| I19 | Secrets never enter prompts, logs, events, telemetry, saved config or agent-visible auth screenshots; owned process trees only are killed. | Sanitized auth transcript and cancellation process audit | not-run |
| I20 | UI distinguishes workspace/team/channel, verified username, visibility, classification, connection/MFA state, local vs remote files and reconnect states. | Empty room never represents auth failure; unread/read and progress remain accurate | not-run |
| I21 | Normal chat uses the same saved connection IDs, transport, host verification, policy and permission engine as Crew UI. | MCP typed operation trace; no model-generated raw SSH or second credential registry | not-run |
| I22 | Natural language/slash commands resolve to typed, destination-explicit operations; reading/selecting does not grant posting or broad context authority. | Ambiguous destination asks selection; public model on Private connection denied pre-retrieval | not-run |
| I23 | Remote commands/jobs have durable invocation IDs and pending/started/unknown/completed states; uncertain execution is not blindly replayed. | Disconnect during acknowledgement; reconnect/cancel/retry semantics | not-run |
| I24 | Linux rootless packaging is portable and refuses unsupported capabilities explicitly. | Home-only install, process ownership, NFS and node-loss matrix | not-run |

## SSH acceptance matrix

| Scenario | Status | Evidence needed |
|---|---|---|
| Known-host noninteractive access | not-run | Authenticated framed bridge stream |
| Two or more gates with distinct host identities and final UID binding | not-run | Separate host-key fingerprints and UID mapping |
| Keyboard-interactive TOTP/Duo at each hop | not-run | Prompt sequence, echo flags, timeout/cancel |
| Passphrase/security-key touch | not-run | Trusted auth surface, no secret persistence |
| Expired auth/cert and MFA-required reconnect | not-run | Reauthentication state, no retry storm |
| Changed host key | not-run | Fail-closed rotation flow |
| Forwarding disabled but approved exec | not-run | Bridge works only when target allows it |
| Forced SFTP-only/prohibited exec | not-run | Actionable incompatibility, no bypass |
| Windows/macOS/Linux differences | not-run | Independent client evidence |

## Required regression catalog

These cases belong below the UI and must accompany the smallest fix that
resolves each defect: forged owner/channel IDs; duplicate request keys;
malformed/oversized/invalid-UTF-8 frames; traversal and symlink escapes; corrupt
or torn journals; policy changes between search and fetch; revocation between
file chunks; stale owner capabilities; direct non-owner requests; replayed
approvals; mismatched enrollment keys; prompt injection in messages; active
HTML/SVG previews; archive expansion; full-disk/quota failures; interrupted
flush/fsync/rename/ack boundaries; and stale-heartbeat competing writers.

Every regression records: final revision, deterministic fixture/profile
sentinels, exact input, expected refusal or mutation absence, observed result,
and evidence path. A UI button being disabled is not sufficient without the
server-side denial and absence-of-effect assertion.

## Current environment and known blockers

- Existing `smoke/aws/run.py` is a useful synthetic two-user identity probe,
  but its own README says it is not production Crew evidence. It uses one
  shared disposable key, two test accounts, one VM, simulated loopback gates,
  and `sudo systemctl restart` during the test.
- The planned three-user fixture, real broker, actual dev-app workflow and
  provider positive/negative paths have not run on this revision.
- The repository's broad no-expansion caution threshold is 200 GiB free; this
  run uses a bounded 30 GiB stop threshold. The temporary target and private
  copy-on-write UI dependency tree are in use without deleting user caches.
- AWS has a verified three-account fixture prepared by the AWS lane, but the
  source transfer approval needed to move the current broker into that fixture
  is pending. No source credentials or private keys were copied here.
- A separate local-only Docker Linux fixture is reachable at
  `127.0.0.1:56928` with pinned host keys and distinct synthetic users
  `alice=1101`, `bob=1102`, and `carol=1103`; see
  `local-linux-ssh-fixture-report.md`. Alice's fresh UI preparation produced a
  public bootstrap key, but the rootless broker has not yet been started in
  that fixture.
- Post-merge focused broker tests: 20 passed (`crates/biorouter-crew/tests/broker_contract.rs`); hook-policy tests: 35 passed (`crates/biorouter/src/hooks/mod.rs`); managed-admission tests: 10 passed (`crates/biorouter/src/managed/mod.rs`); MCP sampling/routing tests: 4 passed (`crates/biorouter/src/agents/mcp_client.rs`). Core/server/Crew check, fresh `biorouterd` build, OpenAPI schema, UI lint/typecheck/theme/contrast/token, version, and cross-drift checks passed. Clippy currently fails on two product string-slice lints in `crates/biorouter/src/crew/mod.rs:160,272`.
  build: passed. OpenAPI and frontend API generation: passed. UI lint,
  typecheck, theme, contrast and token checks: passed. Visible three-profile
  three-user broker workflow, real provider positive/negative sink checks, and 50-user soak
  remain `not-run`.
