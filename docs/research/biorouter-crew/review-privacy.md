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
