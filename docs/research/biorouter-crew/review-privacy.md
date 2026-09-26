# Independent privacy and feasibility review

Reviewed September 21, 2026: `implementation-plan.md`, `smoke/host_probe.py`, `smoke/run_host_probes.py` and saved institutional probe results. Read-only review; no application build, host command or real-data test. Line numbers below refer to the draft before review corrections.

## Findings

### P1 — Human-only authority must remain inaccessible to private workers too

`implementation-plan.md:197,227–231` correctly states that kernel UID proves an account, not a person, but makes the strong worker-isolation requirement most explicit for public models. A private model's arbitrary shell can also connect to the same UID-authenticated broker, read that user's controller credentials, and submit an owner approval or broaden membership. A model being institutionally approved to process data does not authorize it to approve its own risky action or release data to additional people.

Specify the actual bootstrap and verification mechanism for human-control authority: for example, an enrolled trusted-device approval key with broker-side verification and request-bound signatures, enrollment/replacement controlled through a separate authenticated institutional path, and a local approval surface inaccessible to model tools. Workers receive only run-scoped authority. Both private and public workers must be unable to reach human control credentials/transports or impersonate a fresh human enrollment. If a deployment cannot enforce that distinction, mark the affected human-only operations unavailable rather than claim that `SO_PEERCRED` distinguishes the UI from the agent. This is additional to provider egress/read isolation, not a replacement for it.

### P2 — A publish-time audience check is insufficient when membership later grows

`implementation-plan.md:181,191,217–221` requires all current recipients in channel A to be authorized for a result derived from B. The draft should also explicitly define what happens when a new member joins A later, A becomes team-visible, or a previous member loses B eligibility. Otherwise a once-valid copied result can become visible through history, search, download, replay or cached context to someone not authorized for its sources.

Preserve source-ACL dependencies or an equally restrictive immutable audience/compartment restriction on each derived event and attachment. Apply them on every later read, including new members. Channel invitations and visibility changes must not automatically widen the audience of restricted historical material; refuse the change or retain narrower object visibility with an explicit UI explanation. Revoked grants invalidate in-flight event queues before their next delivery.

### P2 — Institutional probe reports name mismatch as rejection without testing a rejection

`smoke/host_probe.py:108–124` computes `claimed_user != kernel_user`, responds successfully, and labels the result `prototype_rejects_forged_username`. This verifies that Linux exposes the true UID and that the fixture notices a mismatch. It does not perform an authorization decision that rejects the request, nor does it prove a distinct user cannot reach the data. The script's one-account scope is otherwise accurately stated.

Rename the check to a narrow evidence statement such as `kernel_identity_differs_from_forged_claim`, or add an actual denied operation and assert both the refusal and lack of mutation. Describe it as identity primitive evidence, with separate two-account broker tests supplying access-control evidence. Do not silently relabel the historical JSON without rerunning or recording that only a result label changed.

### P2 — Overall probe exit omits SSH exit status

`smoke/run_host_probes.py:64–66` checks JSON booleans, byte hash and SFTP status but omits `entry.returncode` and `ssh_stdio_binary_roundtrip.returncode`. A remote command can emit expected output and then fail, causing the wrapper to report success despite an unsuccessful operation.

Require both SSH return codes to equal zero as well as the status-bearing checks. The saved institutional results were inspected: both hosts currently record zero for primitive SSH, binary roundtrip SSH and SFTP, with matching hashes. Consequently this is a harness robustness defect, not evidence that the recorded current runs failed.

## Positive conclusions and explicit limits

The draft correctly avoids treating SSH encryption, source tests, a same-account socket exercise or an AWS synthetic topology as HIPAA certification or real institutional MFA evidence. It keeps canonical storage single-writer, checks authorization at commit, specifies fsync-before-ack and attachment-before-reference ordering, treats idempotency as admission safety rather than exactly-once external execution, rejects automatic multi-host journal sharing, and separates owner session state from channel projections.

No additional blocking journal-design contradiction was found in this pass. Later implementation must preserve idempotency records/results through snapshots or retention and specify the retry horizon before journal compaction, because pruning those records can turn an old retry into a fresh execution. Crash/power-loss tests, real service-account/two-user deployment, restrictive MFA/jump flows and worker read/egress enforcement remain required; the primitives probes do not establish them.

## Resolution verification

All four findings are addressed in the revised design or probe harness. This is closure of the reviewed planning/evidence defects, not verification of a shipped Crew security implementation.

1. **Human authority — design addressed.** Revised `implementation-plan.md:202` specifies operator/existing-human-device-approved enrollment, protected signing authority, request nonce, exact action/payload binding, policy epoch and expiry. `:232–236` requires separate enrolled-device/session credentials or scoped worker grants in addition to the Unix account, prohibits bare-UID human enrollment/admission, and applies human-authority isolation to private and public workers. It selects human-chat-only operation when that isolation is unavailable.
2. **Future audience — design addressed.** Revised `implementation-plan.md:226` preserves source-ACL dependencies on derived events/attachments and checks every future reader, including new members, search/replay and cached context. Invitations and visibility changes cannot silently expand those objects' audience.
3. **Probe claim — addressed and rerun.** `smoke/host_probe.py:108–123` now reports `claimed_name_differs_from_kernel_identity`, matching its actual observation. The saved report at `2026-09-22T03:19:51.081257+00:00` contains the renamed check as true for both Narrows and Leo; it is not presented as a rejected authorization operation.
4. **SSH status — addressed and rerun.** `smoke/run_host_probes.py:64–67` now requires zero primitive-command and binary-roundtrip exit status. The same saved rerun records zero for both SSH commands and SFTP on both hosts, true primitive results, and matching binary hashes.

The optional journal follow-up is also incorporated: revised `implementation-plan.md:296` defines a retry horizon, retains payload digests and stable results across snapshots/compaction and live operations, and refuses expired keys instead of treating them as fresh mutations. The companion source-investigation document now points to the consolidated plan and matches its all-worker authority and future-reader ACL requirements.

Verification in this follow-up was inspection of the revised files and saved rerun JSON only; no additional host test or application build was launched by the reviewer. The original findings remain above as the review history. No unresolved material finding from this review remains in the plan.

## Linux packaging follow-up — September 22, 2026

A bounded source review of the new Linux broker packaging integration found two P2 validation gaps: failed broker inspection could be hidden by a successful aggregate `readelf` pipeline over the existing binaries, and packaged/oldest-distribution startup checks did not execute the newly shipped broker. Both are addressed in the revised source:

1. `scripts/check-glibc-floor.sh:25` and `scripts/check-linux-runtime-deps.sh:77` inspect each named binary separately. Any failed `readelf` command exits the container with status 2, and the caller exits 2 before aggregation. Filenames are passed as quoted positional arguments. The existing maximum glibc-version comparison, runtime-library allowlist, and package-dependency checks remain in force.
2. `crates/biorouter-crew/src/main.rs:6` provides successful, side-effect-free version/help commands before operational dispatch. Debian and Rocky package smoke checks (`scripts/build-cli-linux-packages.sh:87` and `:108`) now locate the installed broker, require successful version output with its expected prefix, and execute help. The Debian Bullseye boot step (`.github/workflows/rust.yml:509`) also requires an executable broker and runs these commands.

The explicit Bash broker-only recipe in `linux-portability.md` uses the centralized pinned cross-build function and documents its output path. Linux build selection, backend artifact staging, nfpm destination/mode, and the documented ordinary-user remote installation path remain consistent. The GUI intentionally invokes the remote broker rather than bundling a local Linux broker.

Both reported findings are closed at source review. This follow-up did not run builds, scripts, tests, package installations, or remote commands; it does not establish that the new CI/package assertions have passed or that operational broker behavior works on every supported host.

## Ollama empty-stream follow-up — September 22, 2026

Independent bounded source review found no actionable issue in the new `require_ollama_answer` wrapper in `crates/biorouter/src/providers/ollama.rs`. It forwards each decoded message/usage/pending item unchanged and propagates upstream errors through `item?`. Only a clean end of stream without non-whitespace `MessageContent::Text` or a `ToolRequest` adds the new `ProviderError::RequestFailed`; usage, pending-tool notifications and reasoning content do not satisfy that predicate. `MessageContent::as_text` and `Message::is_tool_call` were inspected to verify those variant boundaries. The wrapper does not reinterpret reasoning as an answer.

The integration is confined to Ollama's streaming method; the common decoder, nonstreaming completion and other providers remain unchanged. This review does not establish live-model success or regression-test results, and it does not turn earlier empty or failed Crew model attempts into successful workflows. No builds or tests were run by this reviewer.

## Tool-outcome room projection follow-up — September 22, 2026

Independent bounded source review found no actionable issue in `ToolActivity` and its `project_run_event` integration in `crates/biorouter-server/src/routes/crew.rs`. Requests enter the per-run map only for exact `crew__request` calls with an allowlisted method; response IDs must match that map, and each matched outcome is emitted once. Transport-level errors and MCP `is_error=true` results produce a generic failure message. Successful `remote.execute` reports a job receipt and directs the user to job status, while other successful responses claim only receipt, not successful task completion.

The new outcome messages contain no raw result/error payload or command arguments. Existing requested-path/program summaries are unchanged, and publication still uses the run's existing scope, policy and ACL checks. State belongs to one `execute_run`, whose configuration limits tool calls to 60; it is not shared between users or runs. A publication error propagates out of the runner rather than silently continuing its final-success path. Source review does not prove runtime event ordering, restart replay or GUI behavior, and does not validate model-generated final summaries. No tests or builds were run by this reviewer.

## Personal `/crew` routing and cancellation follow-up — September 22, 2026

The bounded source review of the pending changes in `ChatInput.tsx`, `CrewView.tsx` and `routes/crew.rs` found issues that were corrected before this record. The initial routing draft let `/crew` with attachments fall through to ordinary model submission; the final handler consumes the exact command, keeps files/images/reference chips in place with an actionable refusal, and clears the consumed command's ref synchronously before navigation. Enter, Send and sole-command picker selection navigate before model/steer/queue gates. Existing-session identity is preserved; navigation neither creates a session nor grants access. Ordinary non-command text retains its send gates. The separate grant endpoint still requires human authority, session reachability, an idle turn, compatible agent/provider and broker admission.

A brand-new personal chat has no session ID to grant. The UI now explains how to resume an existing conversation or start one with a non-sensitive first message before granting access. This is an explicit two-step usability limitation, not an empty-session grant flow or evidence that private data must be sent first.

The cancellation review confirmed a race in which late completion or progress could overwrite cancellation, an unconditional success response after failed persistence, and failure cleanup that could lose the ability to retry unconfirmed revocation. The revised code reserves cancellation and marks its token under the ledger lock, prevents automatic updates from replacing terminal/uncertain outcomes, distinguishes applied, terminal and persistence-failed transitions, and derives the final stream event from stored status. Setup/runner failures retain `cancellation_unconfirmed` when revocation cannot be confirmed. HTTP responses distinguish already-finished work, durable cancellation and uncertain persistence/revocation. The UI permits explicit recovery for pending/unconfirmed cancellation, interrupted work and non-durable outcomes. These changes do not claim termination of an already-started remote process.

No actionable issue remained in these reviewed source deltas. Security and correctness conclusions are limited to the inspected paths; no material performance or maintainability issue was identified. Race tests, model behavior and native UI acceptance belong to the execution reports. This reviewer ran no tests, builds or probes and made no product edits.

## Selected-channel discovery follow-up — September 22, 2026

The bounded source review covered the pending `crew/mod.rs` and `agents/crew_extension.rs` changes against `373acdf5`. Admission now includes `source_channel_ids` from the same source list submitted to the successful broker `run.create`, plus `history_channel_id` identifying the existing destination-only history. The connections tool returns only that conversation's bound connection and source IDs; it does not enumerate other saved aliases, teams or channels. The added instructions direct the model to `context.manifest` or per-channel `messages.search`, without adding another content fetch.

The review identified a local stale-metadata window: `CrewClient::call_tool` already checks dispatch policy, but its later scope/connection snapshots used separate locks. The final `agent_connections` now calls `validate_worker_scope(session, &s, &c)` immediately before serialization (`crew/mod.rs:965`). That check rejects a locally expired scope, replacement scope, changed policy epoch/workspace identity or incompatible public/private state without fetching content. It supplements the caller's tier/capability check; it does not turn cached metadata into proof of live broker membership or TTL validity.

Actual history/search/manifest requests retain their existing worker checks and broker validation of UID, active principal, revocation, expiry, policy and source membership. History/search require a granted `channel_id`; the manifest returns at most 200 visible messages across the granted sources, not 200 per channel. Broker provenance/visibility filtering is unchanged. Source IDs are discovery metadata and confer no new read or posting authority. No disclosure of an ungranted alias/channel or added automatic history retrieval was found in this delta.

No actionable source finding remained after verifying the local validation guard. This does not establish that a model will retrieve the relevant selected-channel evidence or answer correctly, and it does not upgrade an earlier retrieval failure into a missing-grant defect. Test, broker-trace and graphical workflow results remain separate execution evidence. This reviewer made only documentation changes and ran no tests, builds or probes.

## Scoped Python script opening follow-up — September 22, 2026

Independent source review of the pending 11-line change in `crates/biorouter-crew/src/remote.rs:793` found no actionable security or correctness issue. The new rule permits `ioctl` only when argument 1's low 32 bits equal `libc::FIOCLEX`. The installed seccompiler 0.5.0 source confirms that `Dword` equality compares that low word. Linux declares the ioctl command as an unsigned 32-bit integer and handles FIOCLEX centrally by setting close-on-exec on the existing descriptor, without using the third argument or reaching a device-specific handler. Ignored upper bits therefore cannot choose another operation. This is equivalent to a descriptor-flag change already possible through the unchanged `F_SETFD` allowlist. See [Linux ioctl implementation](https://raw.githubusercontent.com/torvalds/linux/master/fs/ioctl.c).

Other ioctl requests still receive the filter's default EPERM. The fcntl allowlist, architecture validation, full Landlock ABI 3 requirement and filesystem grants remain unchanged; socket/connect and fork/clone syscalls are still absent from the allowlist. The helper still clears its environment, applies no-new-privileges and resource limits, and confines itself before executing agent-controlled argv. The change neither admits another filesystem path nor grants networking or process creation. No material performance or maintainability issue was found in this bounded delta.

The proposed diagnosis is consistent with [CPython 3.13 file utilities](https://raw.githubusercontent.com/python/cpython/3.13/Python/fileutils.c): `_Py_fopen_obj` makes the script descriptor non-inheritable, its ioctl fast path uses FIOCLEX, and EPERM returns an error rather than taking the fcntl fallback. This source observation does not establish the fixture's actual failing syscall or a successful repaired workflow. Script execution, exact output/attachment verification, FIOCLEX positive and other-ioctl/network/process/path negatives remain execution-lane evidence. This reviewer ran no tests, builds or probes and made no product edits.

## Opaque message cursors follow-up — September 22, 2026

The bounded source review covered the pending changes in `broker.rs`, `agents/crew_extension.rs`, `CrewView.tsx` and `crewApi.ts`. The broker replaces the global journal sequence on every inspected message wire surface with that message's existing random UUID: history/search, context manifests, post/projection acknowledgments, ordinary cached mutation responses and revoked-run terminal replays. Internal message ordering, journal records, cached results and read watermarks remain numeric, preserving the existing storage/replay format. Cached message results are projected from current state after checking visibility rather than returning their stored numeric sequence.

Cursor resolution requires the requested channel and current message visibility, including source-channel dependencies and worker scope. Missing, wrong-channel and ACL-hidden string anchors receive the same `stale_cursor` refusal; numeric legacy anchors are explicitly rejected rather than silently ignored. Empty history pages return a previously validated `after` token or null. Read updates retain the internal maximum watermark; snapshots and acknowledgments expose only the most recent currently visible UUID at or before that watermark, or null. The MCP schema and desktop message/read-position types now use opaque strings without numeric comparisons. No additional message-sequence disclosure or broker authorization regression was found in these inspected paths. The separate workspace-wide `policy_epoch` remains visible and can reveal policy changes; this patch does not establish complete activity-metadata isolation.

**P2 — stale-page recovery also clears an unrelated draft — resolved at source review.** In the initial UI draft, the polling error handler reset `historyBefore` for `stale_cursor` but still cleared `snapshot`. The snapshot-dependent selection effect then cleared `channelId`, whose effect cleared the composer body, attachments, references and selected context. Losing access to a pagination anchor's source channel could therefore discard an unsent draft even while the destination remained authorized.

The revised shared `refresh()` catches this refusal only for a non-null older-page anchor and the current request generation. It clears stale messages and resets `historyBefore` while retaining the fresh authorized snapshot and channel; the changed paging state triggers the existing refresh effect to fetch the latest page. Composer content, attachment/reference selections and selected context are untouched. Both timer and manual refresh use this shared path. Actual channel removal still clears the selection explicitly, and other current-generation history failures still propagate to the fail-closed clearing handlers. This closes the reported draft-loss defect at source review; no actionable finding remains in the bounded cursor delta.

This was source review only. No tests, builds or probes were run by this reviewer, and storage compatibility, pagination/restart behavior and draft preservation are not presented here as executed acceptance results.
