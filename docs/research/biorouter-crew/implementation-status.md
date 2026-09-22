# BioRouter Crew implementation and acceptance status

Status is recorded against the final revision under test. A lower-level fixture
or unit test never upgrades a real-app, cross-platform, or institutional gate.
Each row must have an evidence path before it can be marked `pass`.

Legend: `not-run` = no execution yet; `blocked` = an external prerequisite is
missing; `fail` = the observed workflow failed; `partial` = only part of the
required evidence exists; `pass` = the stated behavior was observed on the
recorded revision. A later change requires the affected checks to be repeated.

Current product HEAD is `906bf68b`; native and immutable Linux ARM64 CLI/daemon builds and the full local gate pass. Exact artifact hashes are below. The final committed `906bf68b` desktop suite passes 550 files: 6,256 tests passed, 19 skipped and zero failures (6,275 total), in 161.42 seconds Vitest/161.85 seconds wall time. The log is `/private/tmp/crew-ui-vitest-final-906.log`. Bounded pinned-current-artifact watch, file-mode/stale-capability and task-mode checks now pass; the complete matrix remains open.

Earlier matched artifacts have bounded CLI collaboration, MFA, file/resume, helper, selected-context, personal MCP, revocation and API privacy passes. They do not establish complete current-artifact acceptance, graphical privacy or mixed GUI/CLI parity. Five bounded model self-test attempts did not execute the required assertions and are not passes. Workflow/test commit `0fb6cd13` passes all five isolation tests, strict Clippy and formatting; product artifacts remain `906bf68b`. The final model attempt verified the pinned CLI/daemon paths and versions but guessed a nonexistent localhost:8080 MCP HTTP endpoint instead of using the Crew tool. It produced no `crew__request`, four required native help/version results or mutation audit. No further retries are planned; detailed attempt history remains in validation. The latest mismatched-artifact watch attempt is invalid for product conclusions; the original concurrent-post/pause and initial context failures remain unexplained. [Validation](validation-report.md) and [CLI evidence](evidence/crew-cli-observer-context-20260922.md) preserve the detailed history.

[Draft PR #366](https://github.com/BaranziniLab/biorouter/pull/366) remains at hosted head `3145dfc5`, with 22 successes and one skip. Later product `8d6c2ae4`, transfer `3cbbf210`, observer `37f2684a`, stop fix `6c9c4ea6`, current `906bf68b`, documentation `bd8c3970` and screenshots `b90939b7` remain local. Automatic review rejected screenshot export through `git push`; publication choice is pending. AWS product-source export and native CUA approvals are separately pending. No rejected action was retried through a workaround. Windows native qualification remains open despite source integration and cross-compilation.

### Artifact provenance

| Artifact / evidence role | Source | SHA-256 / qualification |
|---|---|---|
| Historical macOS daemon (retained clients/owned daemons now closed) | `aac5f4ac` | `1ce50cb31f63dca70c7bb25c571facd1672fe9271801ddbf5e82f5785483e407`; version `1.91.1`, help, three-client reconnect and four API boundary cases recorded. |
| Immutable native CLI | `906bf68b` | `4095d6e2b902be43273535de61fd3eb323a757eb1c49b8470cf67e61ff7437a8`; native build and full local gate pass; bounded current-artifact watch/file/task passes; broader matrix open. |
| Immutable native daemon | `906bf68b` | `45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9`; native build and full local gate pass; Linux build and bounded UID-1101 lifecycle now pass; bounded current-artifact watch/file/task passes; broader matrix open. |
| Immutable Linux ARM64 CLI | `906bf68b` | `ff3bfb9ba5a872912c806a152ef57e69761984411ea881446a36ff0cae53c58f`; GLIBC 2.39, version/expected-mode help and bounded UID-1101 lifecycle pass. |
| Immutable Linux ARM64 daemon | `906bf68b` | `3c6049913688cb7d72b7b41874ac2d833411b51693a0bef5dfcb0859f9041bf0`; stable profile/new instance and wrong valid-format proof 403 pass in disposable fixture. |
| Earlier macOS CLI artifact | `aac5f4ac` | `af43d934f73ae14c3708da5eb5b9febc6ce3516f5fcbc93a9e8a4e7fb24cc44c`; version `1.91.1` and help passed. |
| Previous three-user Linux ARM64 broker | `3145dfc5` | `a38b34be38201fe6a1aacd21451d4c8b9311cc3478fd9d919f36810a037d83f5`; real Linux helper/confinement regression and rootless preserved-state upgrade pass; maximum imported GLIBC 2.39. |
| Current three-user Linux ARM64 broker | Cursor source subsequently committed in `37ac3803`; build provenance in validation report | `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`; 30 passed, one ignored plus exact helper/confinement regression. Now installed through all three ordinary users with preserved state; bounded three-user CLI collaboration, file, context and personal MCP results are recorded below; full current-artifact acceptance remains open. |
| Pinned Linux x86_64 broker | `3145dfc5` | `eefaece19ce99b3ba54f7358c64a6d0ed4efb6788b37a8b080fefe3b7a9cb3e4`; maximum GLIBC 2.30, bounded Bullseye version/help/start/status/bridge hello as UID 65532. Emulated container smoke, not native-host/full-package acceptance. |
| Earlier Linux broker used by boundary/cancellation/50-UID evidence | Historical artifact | `7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759`; those executions do not automatically validate the new broker. |
| Earlier GUI CSV-read daemon / CLI | `373acdf5` | Daemon `6c19b87f661f78a65011b33c00951206aecc14a19cbe75dab6d88a53ae1ca5ef`; CLI `3fe16d8f49650aef87565ba79cb45bd33a8f8e486111b7329c796d50bfa37f87`. |
| Earlier API-positive/PAM daemon | Historical artifact | `1ffa69d980d4664b452b2c492100af96fd4055ba3d0e875f076b331fc83b8d86`. |
| Earlier three-client human / later failed-model daemons | Historical artifacts | Human workflow `8f0113378714acb7308ce36fd506e145ac26009ed454aae31575e4d8530abed9`; `367fe588` daemon `3133646fa1fc6b6369677f0c6b769b7ead5ce1bd145bf803e38bea29d3f71cc3`. |

Exact commands, counts, run IDs, failed attempts and artifact transitions are
retained in [validation](validation-report.md), [UI acceptance](crew-ui-acceptance-report.md),
[provider boundaries](local-provider-boundary-report.md), [adversarial validation](adversarial-validation.md)
and the [coverage map](regression-coverage-map.md). The wrong-format Mach-O
canary is excluded as a test-setup failure. The old manually altered Bob
fixture remains excluded from nominal evidence.

### Requested core and explicit limits

The source implements ordinary-user broker installation/lifecycle, persistent
teams/channels/invitations, profiles and preferences (nickname/avatar/read
positions plus local connection settings), files, owned agents, shared
connection MCP and mandatory public/private admission. Open model/UI workflows
above are acceptance gaps, not evidence that those entire features are absent.
SSH/MFA/jump support is core scope. Per-hop preflight is committed in
`bc3e26fb`; focused checks and bounded native MFA cases pass. Broader
institutional and cross-platform qualification remains open. Authentication
reuse currently lasts until explicit disconnect or SSH termination; no maximum
credential-age lifetime is implemented or claimed.

Some broader plan capabilities are genuinely absent or unsupported: NFS/SMB
canonical storage, institutional data-compartment/provider-realm labels,
approved local private-session persistence, read/download audit records,
retention/compaction and backup/migration/restore tooling, scheduler integration,
arbitrary shell and automatic cross-node failover. Current fixed storage quotas
refuse further writes; they are not a retention policy. Narrows' NFS home and
Leo's Landlock ABI 1 do not meet the current broker/helper requirements. Neither
institution has a deployed acceptance pass, and synthetic boundary evidence is
not institutional PHI qualification. Threads, reactions and message editing
were plan expansions, not required core delivery. The broad I01–I24 partial
labels below include both expanded qualification and these explicit capability
limits; they should not be read as either core completion or wholesale absence
of the requested implementation.

## Feature and interface parity ledger

These states refer to current product HEAD `906bf68b`, with historical evidence labeled separately. The daemon owns connection/authentication, tasks, transfers and observation. Verified observer identity/mode and `expected_mode` guard mutations; optional CLI expectations do not change saved mode, and private-origin restrictions remain independent. Cleanup is receipt-bound. GUI Retry refreshes metadata before observation with abort/generation guards. [Plan §15](implementation-plan.md#current-source-milestone-and-unresolved-review) specifies the shared contracts.

Current native CLI SHA-256 is `4095d6e2b902be43273535de61fd3eb323a757eb1c49b8470cf67e61ff7437a8`; daemon is `45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9`. Immutable Linux hashes are in the artifact table. Native/Linux builds and `just check-everything` pass on `906bf68b`, with registry 61/61 and privacy 21/21 and unchanged source/generated files. Linux ARM64 GLIBC 2.39 version/help and UID-1101 stable-profile/new-instance/wrong-proof-403 lifecycle pass. The final `906bf68b` desktop suite passes 550 files with 6,256 passed, 19 skipped and zero failures; bounded live current-pair watch/file/task checks pass; the broader matrix remains open.

Focused passes are core 17, server observer 6, transfer 7, CLI expected-mode 7 with strict Clippy, UI upload/native five files/22 tests, then CrewView nine tests/typecheck. These scopes overlap. The final full UI 550-file/6,256-passed result covers committed `906bf68b`; the earlier 6,250 result is historical. Earlier suite counts and artifact transitions are retained in [validation](validation-report.md), not summed here.

Bounded earlier matched-pair passes cover bootstrap/vault/lifecycle, strict two-hop encrypted-key/PAM MFA, three-user membership/chat/search/read/refusal, 43-byte cross-user file transfer, corrected 32 MiB restart/resume, exact helper execution/output/job identity, selected context, personal natural-language MCP and revocation. The [CLI report](evidence/crew-cli-observer-context-20260922.md) retains IDs/hashes and failed attempts; [the 50-UID soak](evidence/crew-50-uid-soak-20260922.md) retains exact 1,500-message replay evidence. API privacy is not graphical privacy, cancellation is not proof all remote jobs stopped, and previous-artifact passes do not close new current-artifact replay gates.

All three owned profiles were restarted, unlocked and reconnected with pinned CLI `4095d6e2…` and daemon `45d42240…` from `906bf68b`. Current-artifact watch checks pass: structured baseline identity/body/channel/sequence and one-second liveness; broker SIGSTOP with asynchronous post, SIGCONT after 0.575 seconds, 21 drained lines including that post, Ctrl-C exit 0 and exactly one matching history marker. A separate live watch/post/history check matches the same message ID and body with successful exits. The original older concurrent-post failure remains unexplained; the new bounded pass does not supply its cause. Current-artifact observer admission also passes: 16 admitted, 17th HTTP 429, detach one and admit a replacement, with all owned watchers cleaned up. Adaptive history also passes on `906bf68b`: a new private channel held 20 messages of 59,983 bytes each (about 1.2 MiB). Explicit history limit 200 received `response_too_large`/“Request smaller history window”; default watch adapted and drained 20 direct frames matching every seeded ID/body hash, with no duplicate or missing message and exit 0. The first shape-only parser result was excluded; corrected parsing reran against the unchanged backlog. This does not qualify slow-reader behavior, backlog State fairness or the entire fault matrix.

Current-artifact file guards refuse expected Public against saved Private before selecting a nonexistent path; the matching 43-byte transfer completes with its recorded hash. Saved Public against expected Private and CLI resume reselection mismatch are also refused. The separate stale-capability test holds the exact opaque capability and request ID from a Private file approval, switches the saved mode to Public, then starts the transfer without path reselection or an expected-mode field. It receives HTTP 400, `Local file approval does not match this transfer`; before/after transfer lists contain no receipt. Private mode and connectivity were restored. The CLI reselection refusal is not substituted for this stale-capability result.

Task admission refuses saved Public against expected Private. Provider nondispatch is inferred from the independently reviewed guard order, not an independently measured provider counter. These bounded watch/file/task passes do not close the full fault, privacy, mixed-GUI or platform matrix. The five bounded self-test attempts did not execute required assertions; none passed. The final attempt verified pinned paths but chose a nonexistent MCP HTTP endpoint. Workflow/test commit `0fb6cd13` passes five isolation tests, strict Clippy and formatting; product `906bf68b` is unchanged.

A fresh independent CLI fixture has executed with explicit new credentials; old profiles were preserved after rejected credential recovery. A fresh Alice GUI launch/closure produced no qualified interaction. GUI preparation for product `906bf68b` plus workflow `0fb6cd13` passes main/preload/renderer builds and typecheck; the existing pinned backend pair was checked. No app launch, profile changes or CUA retry occurred, preserving CLI-only conditions. Actual GUI interaction remains unqualified pending native approval. A rejected diagnostic-metadata handoff was reduced to this sanitized outcome and creates no additional acceptance gate. Full GUI/CLI, graphical privacy, Windows native and AWS acceptance remain open. `biorouter crew daemon` is the native daemon command family; the [CLI guide](cli-guide.md) documents usage.

| ID / feature | Shared service state and owner | GUI state | Native `biorouter crew` state | Completion evidence |
|---|---|---|---|---|
| P01 Profile daemon lifecycle/discovery | Runtime/UDS profile identity, descriptor, lifetime lock and identity/Proven-only stop source present | Shared proxy/detach source present; ASCII proof/Proven attach and stale-descriptor correction source present | Same-socket HTTP identity/peer-UID client and launcher source present | G10/G14: no Electron dependency, concurrent attachment, stale/profile mismatch, bounded stop/restart; Windows shared transport absent |
| P02 Human control/bootstrap | Proven-only digest bootstrap and vault manager/init/unlock/lock with fresh-profile guard source present; vault primitive review closed | Native prompt/confirmation and independently held ASCII human proof source present; no persisted secret or attach-key replacement | Explicit no-echo/secret stdin and digest-only startup source present | G12/G14: proof/refusal and worker negatives, lock/restart/corruption, no secret argv/env/descriptor/settings acquisition |
| P03 Connections, identity and SSH/MFA | Daemon PTY source plus existing manager/broker identity/preflight | Thin Electron WebSocket adapter source present | Three-user SSH and bounded strict two-jump/encrypted-key/PAM MFA pass; full matrix open | G10/G11/G14: terminal MFA with Electron closed, same identities, per-hop trust, echo/cancel/reconnect |
| P04 Teams/channels/invitations/ownership/profile | Existing broker authority and daemon operations | Historical bounded three-client evidence; shared proxy now unvalidated | Three-user enrollment/team/channel and unauthorized-channel refusal passed | G10–G12: matching objects/memberships/ownership and denials |
| P05 History/search/context/read/watch | Shared typed NDJSON observer implemented; source review closed, server 5/CLI-client 5 and schema regeneration pass, bounded live watch/detach passes; wider recovery and mixed-interface acceptance open | Own automatic polls removed; typed observer consumer implemented, runtime unqualified | History/search/read markers passed; shared observer consumer implemented, baseline and corrected interrupted-broker detach pass; earlier failure unexplained | G10–G12: exact IDs, scoped cursors, source retrieval, watch/revocation/reconnect |
| P06 Human posts, references and retries | Existing broker mutation/idempotency and new shared client source | Historical chat/reference evidence; new adapter unvalidated | Three-user posts passed; reference/retry acceptance open | G10–G12: stable retry IDs/receipts and no duplicate/unauthorized publication |
| P07 Upload/download/resume | Streaming transfer capabilities and durable receipts source present; independent review active | Thin React/native picker, overwrite confirmation and verified image preview source present; Windows helper integrated/unqualified native durability and named Save acceptance open | 43-byte download passes; 43-byte cross-user download and corrected 32 MiB restart/unlock/SSH/watch completion pass; broader fault matrix open | G10/G11/G13: binary hashes, bounded memory, interruption/reselection, path identity and atomic overwrite rules |
| P08 Owned tasks, approvals and cancellation | Existing task ledger/policy plus new bounded shutdown/cancel-owned-runs source | Historical owned-task/Carol processing evidence; new lifecycle unvalidated | Owned-task cancellation and remote-grant revocation confirmed | G10–G12/G14: same run ownership/IDs, follow/approve/cancel, detach and honest interrupted outcomes |
| P09 Personal-chat grants and privacy | Grant/context/revoke endpoints source present; existing scoped privacy remains mandatory | CLI personal MCP/context/revocation pass; historical GUI consent/API negatives remain separate | Native grants/privacy command source present | G10–G12: same effective policy/context, revoke and zero forbidden sink bytes |
| P10 Shared typed API/client and packaging | `906bf68b` native builds and full local gate pass; generated API/source unchanged; Linux build and bounded UID-1101 lifecycle pass; bounded pinned live cases pass; wider coverage and hosted checks remain pending | Shared proxy and thin adapters source present | Full command tree source present on Unix; shared Windows CLI unavailable | G15: matching commands/schema, exact final artifacts/checks and no duplicated business state machines |

[Plan §15](implementation-plan.md#current-source-milestone-and-unresolved-review) records accepted contracts and open review. Existing GUI evidence never validates the new shared adapters or closes native CLI parity.

## Release gates

| ID | Gate and acceptance invariant | Type/evidence required | Status | Owner / next action |
|---|---|---|---|---|
| G01 | Actual BioRouter Electron development app is built from this worktree and three independent app clients drive the visible Crew UI. | Build command, revision, sanitized screenshots/video, action timeline | pass at recorded revision | Clean Alice/Bob/Carol clients were retained simultaneously on `8f011337…`; later human/PAM actions have separate evidence. The `373acdf5`/`6c19b87f…` graphical CSV-read control has matched backend evidence. Current `aac5f4ac`/`1ce50cb3…` is installed, and all three restarted clients reconnect with verified identities, `Synthetic Lab` and `People 3` visible (foreground sessions Alice 62125, Bob 19707, Carol 2183). This is connection/UI evidence; full agent workflows remain under G04/G05. |
| G02 | Three real Unix accounts use distinct UIDs, home directories, SSH identities and profiles; no shared key or copied provider credential. | Disposable AWS fixture report with account/key/UID mapping | blocked for AWS; local evidence complete | Local Linux users 1101/1102/1103 used distinct fixture keys and isolated app profiles. AWS accounts were provisioned, then the fixture was cleaned up at 13:40 UTC without product transfer. Source/binary export approval remains absent; a later AWS product run needs approval and a fresh fixture. |
| G03 | Alice hosts the broker from a user-writable home; product install/start/restart/recovery is rootless and creates no system account/group/service/global SSH change. | Privilege trace, filesystem manifest, process UID, install/recovery transcript | partial | The new `a38b34be…` broker was installed through each ordinary UID and restarted as Alice (1101), preserving workspace/key/node/socket and journal/history. The pinned x86_64 `eefaece1…` passes bounded service smoke as UID 65532. Earlier wrong-node/torn-tail, quotas and injected journal write/fsync failures retain their own evidence. Power loss and the full storage-fault matrix remain unexecuted; NFS is unsupported. |
| G04 | Alice, Bob and Carol complete connect/enroll/general/analysis/files/owned-agent/context/revocation/recovery workflow with human chat before model selection. | Three-app evidence bundle and event IDs | partial | Human chat, files, restricted invitations and accepted ownership transfer have UI evidence; all three reconnect on daemon `1ce50cb3…`. Earlier owned tasks were text-only, and selected extra-channel scope was granted but KAPPA was not retrieved. Bob completed personal consent/navigation on `6c19b87f…` but his MCP turn failed empty. Run `4d7554d4…` issued correct helper argv and failed with EPERM; the FIOCLEX fix now passes exact-helper Linux regression on `a38b34be…`, with Carol’s rendered processing/attachment positive independently corroborated against typed records and UID-1103 disk bytes. Live HTTP cancellation has bounded ownership/event/ordering passes. Personal MCP, cross-channel retrieval, unconfirmed revocation runtime retry and the remaining recovery matrix stay open. |
| G05 | Real provider positive pass uses the actual BioRouter provider path and a public-safe fixture; negative pass uses a designated synthetic public sink and observes zero forbidden canary bytes. | Provider identity, sink digest/bytes, dispatch/tool/context traces | partial | Four API boundary cases passed on `6c19b87f…` and repeated successfully on historical `aac5f4ac` daemon `1ce50cb3…`, with broker `7311f126…`: one allowed public control followed by zero additional requests/canary for workspace-private, personal-private, alias and retained-history refusals. The API calculated-result positive remains on `1ffa69d9…`; the backend-matched 31-byte GUI CSV read remains on `6c19b87f…`. That historical daemon's three connections and API negatives do not establish its full model workflows. Bob's prior personal grant succeeded but MCP failed empty; Alice's earlier graphical cross-channel attempt returned stale CSV text without tools. Carol’s rendered GUI processing/attachment positive is recorded separately, with passing independent backend/direct-SSH corroboration; CLI personal MCP now has bounded evidence; graphical personal MCP/privacy acceptance remains open. |
| G06 | 10, 30 and 50 independently authenticated Unix participants exercise bounded load; the 50-user target is a 30-minute soak with three graphical clients retained. | Workload seed, p50/p95/p99, CPU/RSS/open files, journal/replay, UI responsiveness | partial | The first standalone 50-user run observed 1,500 acknowledged/read-back messages but lacked complete duration/restart/resource evidence. The second `7311f126…` run records a top-level 30-minute workload, 1,500 exact IDs/body hashes, zero errors/disconnects and exact same-state restart replay. Latency p50/p95/p99 was 172.94/712.79/903.92 ms; broker peak RSS was 1,262,800 KiB. Per-participant duration timestamps were absent. Its broker process/workspace was separate from the GUI fixture, even though both used the same binary artifact; this does not prove GUI responsiveness under load. The new `4f11d858…` 30-minute/50-UID run now records 1,500/1,500 acknowledgments and exact readback/restart with zero loss/hash mismatch/errors; [curated evidence](evidence/crew-50-uid-soak-20260922.md) is available; combined-interface/resource qualification remains open. |
| G07 | macOS, Windows and Linux client compatibility is measured separately for OpenSSH/MFA/process lifecycle. | Per-OS run records; no macOS substitution for other OSes | partial | Native CLI fixtures now cover forwarding-disabled approved exec, SFTP-only refusal, changed gateway/final-target keys and two manual PAM reconnects. These used no Crew artifact. The earlier short-path fix passed Electron PAM authentication/explicit close on `1ffa69d9…`, not a complete PAM Crew workspace connection. Desktop policy/reconnect cases, institutional MFA and independent Windows/Linux desktop acceptance remain open. New shared IPC is Unix-only. The Windows helper is fully integrated in source; native runtime/durability acceptance remain open; required Windows desktop transfer functionality is not yet qualified. |
| G08 | All critical/high ownership, privacy, data-loss and provider-routing defects are fixed and replayed on one final revision. | Independent adversarial review, regression IDs, retest evidence | partial | `aac5f4ac` has all ten local checks, completed daemon/CLI builds, focused route/discovery/navigation tests and all 22 active hosted checks green. The FIOCLEX correction is pushed in `3145dfc5`, passes all ten local checks plus actual ARM64 Linux helper/confinement regression and pinned x86_64 bounded Bullseye smoke; its hosted checks now show 22 successes and one skip, excluding later local commits. Cursor/draft work is committed in `37ac3803` with bounded focused passes; the refreshed Linux artifact passes its recorded contract/helper subset. Native per-hop preflight is committed in `bc3e26fb`, with seven focused and three affected admission/worker passes plus formatting/whitespace checks. Its legacy optional-field case does not establish old-client runtime support. The final local gate now passes on `8d6c2ae4`; new hosted checks and affected graphical workflows remain outstanding; no one-final-revision acceptance is claimed. Retained platform-client omission remains source-reviewed only. |
| G09 | AWS instances, volumes, security groups, keys and fixture credentials are deleted and independently verified. | AWS cleanup report and post-cleanup resource queries | pass | Current fixture cleanup completed at 13:40 UTC: instance terminated, volume/security group/key pair independently absent, local key files removed. The lifecycle JSON records exact identifiers and checks. |
| G10 | Native CLI completes the supported workflow with all Crew Electron clients fully closed. | Process tree, headless profile/auth/MFA, rooms/messages/watch, files/resume, owned tasks, approvals and restart traces | partial | Isolated native lifecycle passes and all retained clients/owned daemons are closed; three-user SSH/enrollment/rooms/posts/history/search/read/denial passed; owned cancellation/revocation confirmed. Clean 43-byte shared download/hash passes; 32 MiB completed bytes exposed a stale resume response; correction `3cbbf210` passes five regressions and the bounded 32 MiB live repeat; independently matched CLI helper execution also passes; bounded helper/context/personal MCP/API privacy cases pass as recorded below; wider fault/agent/recovery coverage and the remaining pinned `906bf68b` fault/mixed-interface matrix remains open. Complete three-user native workflow validation on the new artifacts; broker lifecycle/help and native SSH probes do not qualify. |
| G11 | Mixed GUI/CLI collaboration among three real Unix users shares identities, connections, messages, transfers and tasks. | Exact IDs/hashes/ownership, bidirectional visibility, policy and context outcomes | not-run | Run after G10 on identified matching artifacts; existing three-GUI evidence remains separate. |
| G12 | Both interfaces preserve Proven-only human authority, owner scope and privacy denial. | Missing/wrong proof, worker human-action attempts, foreign runs, stale grants/cursors, revoked transfer, changed key and zero forbidden sink bytes | not-run | Complete the runtime denial/privacy matrix for the implemented terminal bootstrap and both interfaces; daemon API secret, UID, PTY and discovery are not human proof. |
| G13 | Shared daemon transfers preserve bounded memory, binary integrity and local file authority. | Interrupt/restart/resume, no duplicate publication, source/destination replacement, symlink/reparse, unauthorized overwrite and atomic finalization | not-run | Complete independent review and acceptance of the source file-capability/receipt implementation; retain native Save approval gap separately. |
| G14 | Authentication and shared-daemon/controller lifetime preserve secrets and honest recovery. | Prompt echo/input/resize/cancel, controller exclusivity, GUI/CLI detach, restart, exact-process shutdown and secret sweep | partial | Isolated approval/vault/stop/new-instance locked restart passed; strict two-jump/encrypted-key/PAM MFA, wrong-secret/cancel, retained connection after CLI exit and secret sweeps pass; wider controller/recovery matrix remains open. Validate corrected GUI reopening, exact-ID cleanup, bounded shutdown and shared lifecycle/auth; no weaker gate or model-visible controller credential. |
| G15 | One final revision delivers shared typed services and complete parity evidence. | CLI commands/help, generated schemas, exact binaries/commit, required checks, independent reviews and G10–G14 traces | not-run | Committed implementation still needs final integration checks, remaining reviews and acceptance; neither historical green CI nor GUI-first progress can close this gate. |

## Requirement invariants

The [regression coverage map](regression-coverage-map.md) separates existing
source assertions and recorded executions from the untested parts of each
invariant. The statuses below refer to the full invariant, not a subset. Each is partial because focused implementation/test evidence coexists with remaining acceptance cases or explicit unsupported plan capabilities; none is promoted to a full pass by a related unit test.

| ID | Invariant from implementation plan | Focused check / negative case | Status |
|---|---|---|---|
| I01 | Authenticated SSH discovers a stable workspace UUID; aliases/jump paths reach the same workspace. | Alias identity comparison; wrong workspace descriptor denied | partial |
| I02 | Broker derives UID from the kernel and binds verified enrolled principal; nickname never grants authority. | Forged username/device, recycled UID generation, same nickname collision | partial |
| I03 | Team/channel membership, invitations, discoverability and multi-team membership are explicit records. | Invite does not create SSH account; OS account enumeration is absent | partial |
| I04 | Creator is initial channel owner; only current owner archives/transfers; transfer requires successor acceptance and revokes stale owner capabilities. | Non-owner removal; stale invitation/approval after transfer; immutable creator audit | partial |
| I05 | Human messages and structured agent events share a room; only the run owner can invoke, steer, approve or cancel. | Mention does not create a run; Bob controls neither Alice nor Carol's run | partial |
| I06 | Attachments are opaque, durable, resumable, ACL checked and digest verified; unsupported previews remain downloadable. | Revocation between chunks; traversal/symlink/hardlink/special-file/archive active-content cases | partial |
| I07 | Search/fetch and derived outputs preserve every source ACL and restriction. | Restricted `methods` cannot leak to `analysis` or a later member; policy changes between search/fetch | partial |
| I08 | Effective admission is the intersection of identity, membership, ACL, shared baseline, personal mode, source restrictions, run policy and endpoint/tool scope. | Any one denial wins; unknown label/endpoint fails closed | partial |
| I09 | Private cluster/workspace blocks every public model, embedding, OCR, summary, plugin and saved-SSH path. | UI, slash, natural-language MCP, reconnect, alias and scheduled-job paths; zero canary bytes at sink | partial |
| I10 | Public mode permits only public-safe inputs/tools; Private→Public never relabels existing history/files/caches. | Restricted object ID, retained private history, fresh public-safe run | partial |
| I11 | Policy epoch changes invalidate incompatible grants; Public→Private blocks queued/new work and reports already-submitted work honestly. | Toggle during queue/stream; stale worker grant rejected | partial |
| I12 | Workers run as invoking user's UID/job allocation; unrestricted same-UID host trust boundary is documented and arbitrary tools are withheld when isolation is unavailable. | Process/job owner; no sudo/container/new account/group; unsupported feature has actionable refusal | partial |
| I13 | One canonical ordered journal is the source of truth; records are bounded, checksummed, idempotent and single-writer. | Duplicate key same/different payload, concurrent writer, lock/fencing, oversized frame | partial |
| I14 | Acknowledge only after flush+fsync; crash/replay verifies continuity/checksums/invariants and only quarantines an incomplete final record. | Torn tail vs interior corruption; no skipped revocation; snapshot atomicity | partial |
| I15 | Home/NFS capability is measured for locking, fsync, atomic rename, quota and disconnect; unsupported storage suspends writes. | Controlled disk-full/I/O failure; no silent `/tmp` canonical store or competing writer | partial |
| I16 | Versioned UTF-8 JSONL framing enforces limits, protocol negotiation, opaque cursors, bounded queues and retry horizon. | Invalid version/UTF-8/frame, slow consumer resync, cursor room isolation, expired idempotency key | partial |
| I17 | Attachment chunks use constant memory; commit fsyncs bytes, renames atomically, fsyncs directory, then journals publication. | Failure before/after each flush/fsync/rename/ack boundary; orphan grace sweep | partial |
| I18 | SSH state machine and per-hop trust/MFA are explicit; changed keys, route denial and reauth are distinguishable. | Keyboard-interactive multi-prompt, TOTP/Duo, passphrase/security key, expired auth, changed key, forced SFTP-only | partial |
| I19 | Secrets never enter prompts, logs, events, telemetry, saved config or agent-visible auth screenshots; owned process trees only are killed. | Sanitized auth transcript and cancellation process audit | partial |
| I20 | UI distinguishes workspace/team/channel, verified username, visibility, classification, connection/MFA state, local vs remote files and reconnect states. | Empty room never represents auth failure; unread/read and progress remain accurate | partial |
| I21 | Normal chat uses the same saved connection IDs, transport, host verification, policy and permission engine as Crew UI. | MCP typed operation trace; no model-generated raw SSH or second credential registry | partial |
| I22 | Natural language/slash commands resolve to typed, destination-explicit operations; reading/selecting does not grant posting or broad context authority. | Ambiguous destination asks selection; public model on Private connection denied pre-retrieval | partial |
| I23 | Remote commands/jobs have durable invocation IDs and pending/started/unknown/completed states; uncertain execution is not blindly replayed. | Disconnect during acknowledgement; reconnect/cancel/retry semantics | partial |
| I24 | Linux rootless packaging is portable and refuses unsupported capabilities explicitly. | Home-only install, process ownership, NFS and node-loss matrix | partial |

## SSH acceptance matrix

| Scenario | Status | Evidence needed |
|---|---|---|
| Known-host noninteractive access | pass at recorded revision | Three actual Linux users and Electron clients used pinned OpenSSH host keys and the authenticated broker bridge; see app and Linux CLI reports. |
| Two or more gates with distinct host identities and final UID binding | partial | Native fixture used two independent jump daemons with strict distinct host keys; actual Crew workspace discovery through that route remains unexecuted. |
| Keyboard-interactive TOTP/Duo at each hop | partial | Synthetic final-target key+PAM prompt/cancel and two manual native reconnects passed; each reconnect prompted once and returned UID 1000. No Duo/TOTP provider, per-hop MFA or institutional/desktop reconnect acceptance is established; see the multi-hop/PAM reports. |
| Passphrase/security-key touch | partial | Encrypted-key passphrase plus PAM passed using native OpenSSH; security-key touch and app replay remain unexecuted. |
| Expired auth/cert and MFA-required reconnect | partial | Two explicitly initiated native PAM reconnects each prompted once, returned UID 1000 and logged one accepted session. This does not test expiry/certificates, automatic retries or the desktop state machine; those remain unexecuted. |
| Changed host key | partial | Native two-jump route refused changed Gate A and final-target keys with exit 255 and no target command output. The complete desktop recovery flow and institutional gateway rotation remain unexecuted. |
| Forwarding disabled but approved exec | partial | Native target with `AllowTcpForwarding no` returned the exact approved exec marker with exit 0 through two jumps. No Crew bridge or desktop artifact participated, so product behavior remains to be measured. |
| Forced SFTP-only/prohibited exec | partial | Native `ForceCommand internal-sftp` target rejected exec with exit 1 and `This service allows sftp connections only.`, without the marker. Crew's actionable incompatibility display/no-bypass behavior remains untested in the desktop. |
| Windows/macOS/Linux differences | partial | macOS actual clients and Linux fixture servers have evidence; independent Windows/Linux desktop runs are still required. |

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

- The earlier AWS smoke is bounded transport/UID/storage feasibility, not the
  production Crew acceptance test. Its fixture was independently cleaned up.
- The AWS three-account fixture was prepared, but automatic approval
  review rejected private source transfer because specific source-export
  approval was absent. No product source or binary was exported by another
  route. Source-export approval remains absent. This fixture was cleaned up at
  13:40 UTC; a later approved AWS product run requires a fresh fixture.
- The local Docker Linux fixture is reachable at `127.0.0.1:56928` with pinned
  host keys and users `alice=1101`, `bob=1102`, `carol=1103`. Fixture setup
  provisions accounts; product installation/runtime use ordinary account rights.
  Runtime is cached `rust:latest`, Debian 13.6 (trixie), GLIBC
  `2.41-12+deb13u3`, LinuxKit `6.12.76` aarch64. Current broker `4f11d858…`
  imports at most GLIBC 2.39; pinned x86_64 `eefaece1…` is a separate artifact.
- Current daemon/CLI and new Linux broker provenance are listed above. Hosted
  green is established on pushed `3145dfc5` (22 successes/one skip); newer local commits remain outside hosted evidence.
  Cursor/draft and SSH-preflight focused evidence is bounded as recorded above;
  final combined acceptance is still open. Exact historical checks and model failures remain in
  the linked reports.
- Native Save is blocked on approval for exact signed-app CUA selection.
  Downloaded temporary staging bytes have matching hashes, but no completed
  named Save is established. Carol's task-local `qwen3:8b` processing/attachment has a rendered
  positive with passing independent backend/direct-SSH corroboration; earlier Settings observations
  do not prove Crew's model field is defective. Personal MCP and cross-channel retrieval remain
  acceptance gaps.
- The actual bounded local-model self-test failed with `tool_loop`. The
  `qwen3:1.7b` live Crew task also issued invalid arguments and summarized
  nonexistent results after a denied tool call. These are failed agent
  workflows, not proof that the remote file guard was bypassed.
- Narrows uses NFS HOME and is currently refused by the broker. Leo's measured
  Landlock ABI 1 is below the ABI 3 execution requirement. Neither institution
  has a deployed Crew acceptance pass; existing SSH connectivity is insufficient.
- Resource expansion follows the repository's 200 GiB free-disk threshold,
  memory/swap/thermal checks and at most two internally parallel heavy jobs.
  Current load is bounded; no user caches are deleted to make room.
