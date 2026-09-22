# Independent source review: Crew transport, server and UI

Review date: September 22, 2026. Reviewer: the Astra broker implementation lane, reviewing code authored by other lanes. This is an independent review of those files, **not** independent approval of this lane's own `broker.rs`, `lib.rs`, `main.rs`, journal or protocol implementation. The root lane is reviewing those separately.

Status: **changes required / final revision recheck pending**. Several findings were fixed during this review and their corrected source was reread. The prepare-device onboarding and final pre-execution lease checks have now been reread in source; final integrated runtime acceptance remains outstanding. No tests, builds, application launches, cloud actions or exploit execution were performed by this reviewer. Luna owns test cases and execution. Source inspection does not establish the three-user desktop acceptance or 50-user soak.

## Reviewed scope

Read the saved connection manager and SSH transport, the owned-run server routes and their grant/cancel/publication paths, the Crew MCP extension, the remote helper's path/job/confinement implementation, desktop authentication IPC and the Crew view/file-transfer components. Also inspected the new dispatch gates and diffs for session reachability, SSE, search, knowledge ingestion, copying/export, declassification, aggregate counters and provider binding.

Primary files:

- `crates/biorouter/src/crew/mod.rs` and `transport.rs`
- `crates/biorouter-server/src/routes/crew.rs`, `session_reach.rs`, `session_events.rs`, `session_meta.rs`, `session.rs`, `usage.rs`
- `crates/biorouter/src/agents/crew_extension.rs`, `agent.rs`, `extension_manager.rs`, `knowledge_tool.rs`
- `crates/biorouter/src/session/session_manager.rs`, `chat_history_search.rs`, `knowledge/conversation_ingest.rs`, `privacy/visibility.rs`, `privacy/declassify.rs`
- `crates/biorouter-crew/src/remote.rs` (transport-lane authorship)
- `ui/desktop/src/main.ts` Crew authentication IPC and `components/crew/{CrewView,CrewFiles,crewApi}.tsx` / `.ts`

This was review of an evolving uncommitted integration. A final frozen diff must be reviewed again; it is not valid to combine this report with unrelated earlier green checks and declare the final revision accepted.

## Tracked findings

| ID | Priority | Finding and consequence | Handoff status |
|---|---|---|---|
| R1 | P1 | **First-workspace onboarding has a key-generation cycle.** `CrewManager::save_inner` generates the desktop device key only after requiring an existing workspace UUID and public key. `serve` requires that desktop bootstrap public key before creating the workspace. A new owner cannot follow the supported UI flow without fake placeholder descriptors. | Corrected source reread: human-only `/crew/devices/prepare`, persistent pending identity, credential-store-only private key, exact preparation reuse on save, idempotent completed-preparation recovery, and desktop hosting instructions are present. Luna first-use/restart acceptance remains required. |
| R2 | P1 | **Nondefault broker authority stores could enter a worker's remote scope.** `remote::root` protected only default application directories, while `serve --state-dir` permits another private home location. Selecting such a location could expose the journal and workspace signing authority through file tools or execution. A second workspace hosted by the same UID makes a per-broker-only check insufficient. | Corrected source reread: broker admission/live grant checks reject overlap with its canonical store; the independently authored helper now performs a bounded worktree scan for any `journal.jsonl` + `writer.lock` store, including descendants. This marker-scan fix and the broker-side fix need independent runtime negatives. Malicious external same-UID host software remains outside the claimed boundary. |
| R3 | P1 | **Remote command lifetime ignored the grant expiry.** `start_job` originally used only a 1–60 second requested timeout, so a command admitted just before expiry could run beyond it. | Corrected source reread: deadline is now bounded by `scope.expires_at` and begins before setup. Final source recheck confirms refusal immediately before spawn and after confinement immediately before helper exec, covering setup consuming the remaining lease. Runtime expiry/race coverage remains required. |
| R4 | P1 | **Queued human contributions could retain stale Public labels.** `signed_request` captured connection mode before awaiting the shared transport lock. A queued post/upload could be signed with the old mode after the connection tightened to Private. | Corrected source reread: policy epoch/mode/identity are rechecked after acquiring the transport, and mode/epoch again after the challenge before signing. Live race regression remains Luna's responsibility. |
| R5 | P1 | **Nested manager locks could form a three-way wait cycle.** Signing held a per-transport mutex while requesting the registry; connect held the registry while requesting the transport map; disconnect could retain the map guard while waiting for that same transport. | Corrected source reread: connect drops the registry before inserting transport; disconnect removes transport inside a short guard scope before awaiting close. No concurrent-runtime stress was run by this reviewer. |
| R6 | P2 | **Remote completion was not durably replaced.** `persist_job` used direct `std::fs::write`, risking truncated/corrupt completion records or loss of a known outcome after crash. | Corrected source reread: private same-directory temporary file, file fsync, rename and parent-directory fsync; failure reports `outcome_not_durable`. Filesystem fault/reconnect cases remain unexecuted here. |
| R7 | P2 | **New tasks read the oldest context window.** Admission fetched `messages.history` with `limit:50` and no recent-window flag, leaving tasks permanently grounded in the oldest messages as a channel grew. | Corrected source reread: admission requests `latest:true`. Backward pagination was separately added to broker/UI during integration. |
| R8 | P2 | **Global billing endpoints bypassed the new Crew metadata exclusions.** Session insights/activity were filtered, but `/usage/report` and `/usage/summary` still exposed aggregate private Crew activity/provider/token information to an unproven authenticated caller, including narrow time windows. | Corrected source reread: both billing handlers now require a proven human action. Generated API schema and HTTP regressions remain validation work. |

The numerical priority reflects the concrete security/correctness effect, not a claim that these cases were executed. R1 was a functional release blocker; its source fix still needs genuine first-use acceptance. R2–R5 require targeted negative/race coverage before a security acceptance claim.

## Positive observations

- Device signing remains in the trusted local manager; model requests receive limited run credentials. Generic human protocol routes refuse run-creation methods and typed run admission resolves the actual provider.
- The manager pins the workspace public key and checks a fresh signed challenge; the new signed root-owned node identity is intended to preserve the strictest mode across aliases and workspace UUIDs on the same physical node. Cross-node cluster grouping stays explicit.
- The central extension dispatch gate verifies both the permitted Crew/todo prefix and the built-in platform extension identity, preventing an external extension from gaining authority through a similar name.
- Remote file operations use relative paths, component-wise `openat` and `O_NOFOLLOW`; the execution helper requires full Landlock enforcement, restricts syscalls and strips inherited credentials/environment. Public-provider remote access is denied.
- The bridge checks the live run before the local helper and again before returning output. A post-operation revocation reports possible effects rather than claiming rollback.
- Generic search, ingestion, copy/export and declassification paths now exclude or refuse Crew-scoped conversation material. SSE delivery rechecks reachability. Shared channel projection uses text content rather than hidden reasoning or raw provider internals.
- Desktop upload recovery persists only bounded transfer metadata, verifies size/hash against the server and resumes from the authoritative offset. Downloads verify SHA256, render only supported image types and keep arbitrary files as downloads. Channel polling replaces the authorized view and clears inaccessible content rather than maintaining an offline transcript cache.

## Limits and remaining evidence

Security: promising layered boundaries, but not an unconditional approval while final integration and runtime validation remain pending and adversarial execution is outstanding. Broker/storage/identity code authored by this reviewer requires the separate independent review.

Performance: no benchmark assessment. The delta journal avoids the earlier full-history-per-commit disk growth, but CPU/state cloning, directory scans, 50-client fairness, disk quotas and the 30-minute soak remain measurement gates. The implementation has explicit state/journal/file limits; these must be presented as limits, not evidence of production scale.

Correctness: source-level fixes are visible, but no final-revision compilation, provider dispatch negative proof, MFA test, three-profile UI interaction or fault-injection result is established by this report. Cloud Linux, macOS UI and Windows OpenSSH remain separate validation scopes. Native multi-hop host-key and MFA behavior must be exercised per hop; one final-destination SSH option is not independent evidence for all gateways.

Maintainability: the shared manager and explicit protocol make policy routing inspectable. Mutex ordering, persistence failures and run completion acknowledgments need focused coverage because superficially small changes can alter authority or recovery behavior. Keep the fixed protocol and precise unsupported-operation refusals; do not replace them with ad hoc model-generated SSH commands.

No claim is made here that all source compartment or institutional endpoint rules are implemented merely because `restricted` classification and channel provenance are present. Endpoint affiliation rules, explicit unsupported compartment combinations and every actual model ingress/egress require their own evidence. The accepted host-account trust limitation, unknown outcomes after interruption, lack of cross-node failover and rootless confinement compatibility remain visible constraints.


## Final bounded source recheck

Reread the subsequent integration changes in `hooks/mod.rs`, `agents/mcp_client.rs`, `permission/permission_inspector.rs`, the changed provider/workflow paths in `agents/agent.rs`, `CrewManager::agent_request`, and the enrollment/offboarding forms in `CrewView.tsx`. No additional concrete defect was identified in this bounded source recheck.

- Hook dispatch and individual execution both consult the current Crew scope, including existing agents that predate a grant; prompt-provider resolution also refuses scoped sessions. This covers command hooks as well as model hooks.
- Auxiliary sampling attribution uses trusted weak references to the shared provider plus locally registered session IDs, with external MCP session labels excluded from the authority decision. Provider updates register the session before installation. MCP sampling also checks active local dispatch sessions; smart-approval sampling uses the same Crew refusal gate. Keeping all sessions associated with a still-live shared provider conservatively denies auxiliary sampling when any associated session is Crew scoped.
- Workflow completion goes through the checked provider accessor; reasoning-effort provider reconstruction checks the actual rebuilt provider before returning it. These source changes do not establish runtime race coverage.
- Agent `run.project` requests are normalized to progress and reject requested terminal states. The trusted runner's publication path remains responsible for completion; this prevents an agent progress tool from prematurely revoking its own grant.
- Enrollment explicitly displays the active UID identity and requires add-device intent. Offboarding requires the exact displayed username and sends the principal UUID. The snapshot excludes inactive principals, so subsequent fresh enrollment does not accidentally select an offboarded generation. The broker remains the authority for identity and membership checks.

This recheck does not supersede the outstanding Luna runtime, fault, concurrency, packaging, and acceptance gates above. No tests or builds were performed by this reviewer; the broker changes authored by this lane still require the root lane's separate review.


### Recovery-order finding from the NFS assessment

Parent-confirmed P1 in broker startup: torn-tail quarantine/truncation ran before recovered host UID and pinned writer-node validation. Starting a moved store on a different node could therefore mutate recovery evidence before refusing service. The broker-author lane moved recovered-state parsing and UID validation ahead of all tail repair, and checks a persisted writer-node identity on Linux before repair or any commit. Missing blob-directory creation now also follows identity validation. This is a self-authored correction, not independent approval; root review and Luna's wrong-node/torn-tail unchanged-bytes regression remain required. NFS admission remains refused and no NFS qualification is implied.


## Review of e9a09c6c integration and subsequent Clippy-only manager edits

Read-only source review, no builds/tests, excluding the reviewer's own broker implementation. The two manager Clippy edits preserve validated hexadecimal decoding and the six-byte control-directory digest prefix. Rechecked managed compatibility refusal at Crew start/grant, reply/provider binding and background compaction; auxiliary sampling attribution; human-only Crew routes; session reach before the global privacy opt-out; central tool identity restrictions; owned-run cancellation and agent progress-only publication. The new managed-policy compatibility checks correct the earlier omission: mandatory hooks must refuse Crew admission rather than be silently skipped. Existing runtime evidence requirements still apply.

**R9 — P1, source correction verified; runtime regression pending: queued worker requests can outlive a stricter local cluster policy.** `crates/biorouter/src/crew/mod.rs:1023` validates the captured scope epoch, then lines 1046–1052 await the transport mutex and send using the captured credential without checking current scope/connection policy again. Save/update of another connection alias in the same canonical cluster updates all member epochs and modes (`save_inner`, lines 450–472), while disconnect closes only the edited alias. Thus a Public worker request on alias A can pass its check, queue behind existing A traffic, and still publish with its old valid broker grant after alias B makes the cluster Private. The broker cannot see this desktop-only epoch change. This is the worker analogue of the already-fixed human signing race R4. Required correction: after acquiring the transport mutex, recheck scope/run identity, expiry flag, current connection epoch and mode before sending; recheck responses before delivery when policy changes during the request. A rejected response after a mutating operation must report an uncertain/completed remote effect, not imply rollback. Reproduction is source-derived, not executed. Parent notified; final source recheck and Luna race regression pending.


### R9 correction recheck

Reread `CrewManager::worker_request` and `validate_worker_scope`: the transport mutex is acquired before revalidation; the full expected scope must still match, remain nonexpired and agree with current connection epoch, mode and workspace key/ID. Public grants additionally require current Public mode and unrestricted origin. A second check occurs after the remote response and before returning it. The error explicitly advises inspecting submitted effects, so response rejection does not imply rollback. This closes the reported queued-alias transition in source; Luna's queued/in-flight policy race regression remains pending. No additional concrete defect identified in this correction.

Also reread the remote invocation durability change: after writing and fsyncing the exclusive invocation marker, `start_job` fsyncs each directory from `remote-jobs` through HOME before spawning the helper. Any failure returns before execution, leaving the marker as an intentionally uncertain/non-replayable receipt. The ordering persists both the marker name and newly created parent names before remote effects can begin. Source review only; filesystem fault validation remains Luna's scope.


### Root-authored connection budget and registry transaction recheck

Independently reread the root-authored changes to the broker accept loop and `ConnectionPermit`, plus the manager's `save_inner` and `persist`. This review covers those specific root-authored broker changes only, not independent approval of the surrounding broker authored by this reviewer.

No new concrete defect identified. The accept loop derives UID from kernel peer credentials before counting, atomically admits at most eight connections per UID and 256 overall, and drops rejected streams without incrementing. The moved permit remains alive through `serve_client` and decrements/removes its UID entry on return or unwinding. The shared count mutex is not held while serving clients. A single account can no longer occupy the full global budget; the limits are resource admission bounds, not measured fairness or capacity guarantees.

`save_inner` now holds the registry mutex while modifying a clone, durably persists that candidate, then publishes it in memory. Failures before publication preserve the earlier in-memory mode and epoch, including failed Private-to-Public saves. Prepared-identity retry does not mutate the candidate and retains its existing idempotency check. Persistence orders temporary-file fsync, atomic replacement and directory fsync; on Unix it also collects and fsyncs newly created ancestor directories through the first existing parent. As with any rename followed by a failed directory fsync, a returned error can leave an uncertain on-disk outcome; this review does not claim rollback after replacement. Luna's quota/RAII and filesystem-fault regressions remain necessary. No builds or tests run by this reviewer.


### Enrollment diagnostics and UI polling recheck

The trusted manager now reads its signing credential and verifies the derived public key/device fingerprint before requesting the challenge, avoiding key-store delay inside the nonce lifetime. Bootstrap/enrollment payloads must match that saved public identity. Existing post-lock policy checks remain. The broker splits the same device binding, public-key fingerprint and expiration predicates into distinct diagnostics; challenge removal still occurs first, expiration comparison is unchanged, and Ed25519 strict verification still binds workspace, kernel UID, nonce, method and params. No authentication weakening identified. The manually altered Bob fixture is not evidence of a product cryptographic failure.

**R10 — P2, source correction verified:** `CrewView.tsx:583–584` displays `error || refreshError`, but its Dismiss handler clears only `error` and its SSH-help predicate still tests only `error`. A polling-only error therefore cannot be dismissed and cannot show matching SSH recovery help. Clear the displayed error state (or both) and use `visibleError` for recovery-help matching. The separation otherwise preserves action errors across successful polling, while a successful refresh clears the polling error. No tests performed; parent notified.

R10 final source recheck: the current Dismiss handler clears both `error` and `refreshError`, and SSH recovery-help matching uses `visibleError`. Both reported defects are corrected in source; no tests were run by this reviewer.

## Integration helper extraction review

The transport Astra lane independently reviewed the root lane's eager compaction, workspace listing and observer-stream helper extractions. Scope exclusion remains before pagination counts, per-frame reach checks remain before delivery, subscription still precedes the initial snapshot, and the observer/compaction guards retain their original task lifetimes. No actionable source finding was reported. The root lane also reviewed the server run extraction: the cancellation token, turn guard and concurrency permit remain owned through run completion; the turn guard is dropped before the terminal event and deregistration. These are source findings, with Luna responsible for the final build and behavioral regressions.

## Uncertain task starts and refresh feedback

The UI preserves a task request ID through an uncertain network retry. A typed `crew_start_outcome_unknown` now retains that attempt across route navigation, blocks an ordinary resubmit, and requires an explicit human inspection acknowledgment before creating a new attempt. Root Astra reviewed the source change separately from its UI author. Background and manual refresh use their own error state, so polling cannot erase an action refusal. Luna's component regressions use different generated IDs to distinguish retry from deliberate restart, real interval polling through failure/recovery, and an unmount/remount check. All four cases passed; they do not substitute for actual remote side-effect or app workflow evidence.


## Bounded follow-up review against 5fbd74f7

The independent Astra architecture/privacy lane reviewed the subsequent product delta on September 22, 2026 without editing code or running tests. No actionable new finding was reported in this bounded review.

- The broker rejects a new mutation while poisoned before invoking filesystem-changing handlers; an already committed, authorized idempotent response remains replayable. An initially uncertain blob commit retains its file because a complete journal record may survive failed synchronization. Unreachable-file sweeping remains unimplemented.
- Unix OpenSSH control paths use a private, no-follow, UID-validated directory and owned-socket validation. The fixed short path leaves room for OpenSSH's temporary suffix independently of long isolated profile paths.
- An omitted tool connection ID is derived only from the existing session grant. Explicit IDs still pass ownership, scope, epoch and privacy validation; admission metadata does not grant additional authority.
- The authentication component buffers only bounded early events for the subsequently returned session, ignores cancelled callbacks and disposes a late-created session after explicit close. Route navigation intentionally retains the owned master.

This is source review of that delta, not a repeated full-PR audit or runtime/cross-platform acceptance. Luna's focused regressions and refreshed app runs are recorded separately.

## Windows SSH subprocess integration

Root Astra independently reviewed the transport Astra follow-up prompted by the Linux source census. Both the bridge connection and owned control-master exit now construct a named Tokio command and call the existing `prepare_agent_child_command` immediately before spawning. That helper applies Windows console suppression and removes daemon-private credentials. The existing SSH arguments, explicit isolated-profile configuration, piped/null streams, kill-on-drop and bounded wait remain unchanged; no environment assignment follows the scrub. Unix broker/helper spawn sites remain platform-gated. This is source review; the census tests and refreshed runtime acceptance are recorded by Luna separately.
