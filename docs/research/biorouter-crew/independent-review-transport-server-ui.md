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
