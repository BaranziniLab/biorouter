# BioRouter Crew implementation and acceptance status

Status is recorded against the final revision under test. A lower-level fixture
or unit test never upgrades a real-app, cross-platform, or institutional gate.
Each row must have an evidence path before it can be marked `pass`.

Legend: `not-run` = no execution yet; `blocked` = an external prerequisite is
missing; `fail` = the observed workflow failed; `partial` = only part of the
required evidence exists; `pass` = the stated behavior was observed on the
recorded revision. A later change requires the affected checks to be repeated.

Published `5455ebf9` retains its full-gate, native-pair and Linux production build/UID lifecycle passes. Merged checkpoint `3dac3695` incorporates `origin/main` (`06c9639a`); API export and docs are committed in `8be945c2`. The merged desktop suite passes 562 files/6,366 tests/19 skips, plus typecheck and 60 affected tests. Runtime-qualified source `532c3b7d954928b3ace92495d7a3e159efae6b9c` commits native continuation recovery and observer corrections; valid fresh-process CLI 11 and daemon-client 16 tests pass, alongside observer 12; the full `just check-everything` gate and ordinary non-test native debug build (2m59s) now pass. The immutable mode-0555 arm64 1.91.1 pair is in `/private/tmp/biorouter-crew-artifacts-532c3b7d/`; Linux 532 build/lifecycle, actual PTY continuation and bounded observer replay now pass. Hosted baseline `7ab40c817436a01df355336a264c17e5b8d5a5e7` passes Vitest/frontend and macOS Rust. Linux integration has three test-setup failures (FIFO/private-temp handling, transport child console census and transfer sandbox declaration); three Windows SSH fixture grammar failures persist. Following changes are test-only: portable temp paths, test-child preparation, sandbox declaration, RUNNER_TEMP-first fixture selection, safe Windows normalization and diagnostics. The original Windows failing path was not captured; no extended-path-prefix cause is established. Test-only portability corrections pass focused integration 42/42 (runtime 7, MCP census 20, server census 5, transfer 10) and core 22/22 (transport 12, SSH 10). The full `just check-everything` gate passes, including registries 61/61 and 21/21, with no generated API diff. Hosted Windows/Linux follow-up awaits the next commit/push; hosted CI is not yet green. The `7ab40c81` ordinary native pair builds successfully; supported SSH authentication and exact Unicode history (including U+E0041 JSON surrogate/text-safe output) smoke pass. Its daemon is byte-identical to `532c3b7d`; broad end-to-end results keep their original `532c3b7d` scope.

On immutable `5455ebf9`, deterministic shared-CLI create-only/grant/tool/stream/revocation passes with zero provider dispatch after revocation. Actual Ollama `qwen3:8b` natural Crew discovery/history/context and projection receipt also pass: an initial missing-body request was correctly refused and the model corrected it once. Bob and Carol independently observed the same receipt after supported fresh TTY authentication. The separate `qwen3:1.7b` plain-marker result remains a narrower pass. [Shared-daemon evidence](evidence/shared-daemon-acceptance-20260922.md) and [Linux evidence](evidence/linux-5455ebf9-20260922.md) retain exact artifacts/results; neither upgrades G10/G11 to complete.

A larger `5455ebf9` observer test exposed a real defect: 60 × 50 KiB messages and a six-second reader pause yielded 12 messages then EOF. This is distinct from the resolved shell-control fixture stall; a smaller probe does not qualify backpressure. The independently reviewed correction committed in `532c3b7d` drains queued data then provides a last-accepted-cursor reconnect fallback, preserves authoritative privacy errors and handles normal-expiry internal cancellation. Twelve focused server observer regressions pass; native pair/full gate and live 60-message replay pass on `532c3b7d`: exactly 60 IDs, no duplicates/missing markers, bounded concurrent post/read fairness and replacement-slot admission. Membership and derived-source refusal include a positive visibility control and zero provider dispatch after revocation. The full queued-derived-observer race matrix remains open. Native pending-continuation takeover/abandon/leave is independently reviewed and committed in `532c3b7d`, using Proven/exact-generation APIs without lease exposure or automatic resubmission; valid fresh-process CLI 11 and daemon-client 16 regressions pass. The full gate/native pair, actual PTY leave/abandon/takeover and Linux 11/16/12 focused suites, non-test build (6m43s) and UID 1101 lifecycle pass on `532c3b7d`. See [current native acceptance](evidence/shared-daemon-532c3b7d-20260923.md) and [Linux evidence](evidence/linux-532c3b7d-20260923.md). The failed 9/11 fixture and discarded unauthorized bypass are excluded from acceptance.

Final `532c3b7d` natural-model acceptance passes: Bob’s own `qwen3:8b` session used connections/history/run.project, and Alice/Carol each retrieved the seeded fact and receipt exactly once. The file flow also passes: an initial starting transfer ID had no blob, so premature posting was correctly refused; supported watch completed the upload before posting, then Bob/Carol independently read, downloaded and watched completion with matching SHA-256 `4fd7fd53f52ec3ba906e93b9f6f4ecf7690c1e7b8f703cb5c3836952f3fee4cf`. Exact session/message/transfer/blob IDs are retained in [current acceptance evidence](evidence/shared-daemon-532c3b7d-20260923.md); transfer and blob IDs are not interchangeable.

Earlier transfer, lifecycle, MFA/helper/context/privacy evidence retains its original scope in [validation](validation-report.md) and [CLI evidence](evidence/crew-cli-observer-context-20260922.md). Historical broken-pipe/server-hang causation remains unproven. Source equivalence does not requalify binaries.

All 26 visual files and the original-history backup remain local only and are excluded from the published branch. The fast-forward source/text push succeeded; publication is no longer blocked. [Source-only history provenance](evidence/source-only-history-20260922.md) maps rewritten revisions without reassigning earlier binary evidence. AWS product-transfer and native CUA approvals remain pending. Full CLI model/MCP, mixed GUI/CLI, observer slow-reader/fairness, broader privacy/fault/recovery and native Windows acceptance remain open. Shared IPC is Unix-only; standalone local conversations remain separate. Five earlier model self-test attempts did not execute required assertions. Unsupported institutional platforms and excluded fixture evidence remain as recorded.

### Artifact provenance

| Artifact / evidence role | Source | SHA-256 / qualification |
|---|---|---|
| Current validated native CLI | `7ab40c81` | `ffa2f599621159be35a9efbf9c6f74c0ce5f3497d72da41d0d6bcece29c09624`; ordinary native build and supported SSH/exact Unicode-history smoke pass; broad `532c3b7d` end-to-end results are not reassigned. |
| Prior runtime-qualified native CLI | `532c3b7d` | `edc9a59ba6afbc37df6b19d09070b3c931f4bd2a6de91f5f3c7965bd165efdef`; non-test debug arm64 Mach-O 1.91.1, mode 0555; full gate/build and bounded PTY/observer/membership/provider/derived-source acceptance pass; final bounded three-user file and natural `qwen3:8b` replay pass; broader matrix remains open. |
| Current validated native daemon | `7ab40c81` / identical `532c3b7d` | `4a1082ec9e1e0cc0b464c8cd608156a21cd24544c27cd87a2d42e7f84b16185c`; non-test debug arm64 Mach-O 1.91.1, mode 0555; same-source Linux build/lifecycle pass, broader platform matrix open. |
| Prior validated native CLI | `5455ebf9` | `b329b6ad161e9b6722ef6b08ebef9aab3df4462c25c516a3206f051525fc7bdc`; arm64 Mach-O 1.91.1, mode 0555; full gate/release build, bounded deterministic tool/revocation and natural `qwen3:8b` Crew receipt acceptance pass, with independent Bob/Carol visibility; broader cases and later-fix replay remain pending. |
| Prior validated native daemon | `5455ebf9` | `e3bcbf581b4ec06f4a24e7a76a27f2843c36e9ef3e8b100087ab85021d5d5498`; arm64 Mach-O 1.91.1, mode 0555; merged `3dac3695` not covered. |
| Current validated Linux ARM64 CLI | `532c3b7d` | `35c52efd390166f59a48325d93211b6580feaf14efb9d129e162c0a900af2d73`; non-test debug build, GLIBC 2.39, mode 0555; focused suites and UID 1101 lifecycle pass. |
| Current validated Linux ARM64 daemon | `532c3b7d` | `81b7a2e3e4f62161490f04535bf11308c3c03aa96b305f27b2cf35d8a9ce8318`; non-test debug build, GLIBC 2.39, mode 0555; same-profile/new-instance and wrong-proof refusal pass. |
| Prior validated Linux ARM64 CLI | `5455ebf9` | `63d21cdd67b8297e8455f51aa3412d27cf982a7a5903a08897dd4eb61d0f5050`; non-test debug build, GLIBC 2.39, mode 0555; UID 1101 lifecycle pass. |
| Prior validated Linux ARM64 daemon | `5455ebf9` | `f3107571d2379c9e5fafcb8b2533bdb4d62b92144b10269d834a511e646e4c96`; non-test debug build, GLIBC 2.39, mode 0555; same-profile/new-instance and wrong-proof refusal pass. |
| Historical macOS daemon (retained clients/owned daemons now closed) | `aac5f4ac` | `1ce50cb31f63dca70c7bb25c571facd1672fe9271801ddbf5e82f5785483e407`; version `1.91.1`, help, three-client reconnect and four API boundary cases recorded. |
| Prior immutable native CLI | `4a2e190b` | `0c1659478c1de91794170e311770abb6e20663b3a358cab68326f76ef4a6429e`; arm64 Mach-O 1.91.1, mode 0555, native build/full gate pass. |
| Prior immutable native daemon | `4a2e190b` | `41ccbf13d4cc6088c340204a32394234f8f0ff7bec45601f9f19fa236ade6203`; arm64 Mach-O 1.91.1, mode 0555, native build/full gate pass. |
| Prior immutable native CLI | Product `9a3957d6`, docs `d3498ecd` | `9bbedb34c349e3b637c8b8e01e6e4e50807ffa86bf1bcfaa1562fc3c64195c5f`; arm64 Mach-O 1.91.1, mode 0555, native build/full gate pass. |
| Prior immutable native daemon | Product `9a3957d6`, docs `d3498ecd` | `50eefaaebad298679d9ad525a98a911941ea8ff62c462033a389fcd3351b005f`; arm64 Mach-O 1.91.1, mode 0555, native build/full gate pass. |
| Prior immutable native CLI | `906bf68b` | `4095d6e2b902be43273535de61fd3eb323a757eb1c49b8470cf67e61ff7437a8`; native build and full local gate pass; bounded current-artifact watch/file/task passes; broader matrix open. |
| Prior immutable native daemon | `906bf68b` | `45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9`; native build and full local gate pass; Linux build and bounded UID-1101 lifecycle now pass; bounded current-artifact watch/file/task passes; broader matrix open. |
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

Broad runtime-qualified `532c3b7d` files remain pinned in `/private/tmp/biorouter-crew-artifacts-532c3b7d/`; prior `5455ebf9` files remain in `/private/tmp/biorouter-crew-artifacts-5455ebf9/`; historical files remain in `/private/tmp/biorouter-crew-artifacts-4a2e190b/`; the preceding handoff was `/private/tmp/biorouter-crew-artifacts-9a3957d6/`; [validation](validation-report.md#current-full-gate-and-native-artifact-handoff-2026-09-22) records exact commands and scope. The `906bf68b` UI/Linux evidence remains prior. Historical product `9a3957d6`/docs `d3498ecd` passes the full desktop suite: 550 files, 6,260 passed, 19 skipped, zero failed (6,279 total), 160.06 seconds Vitest/160.46 seconds wall; log `/private/tmp/crew-ui-vitest-d3498ecd.log`.

Fresh `9a3957d6` connection refusal was resolved as fixture setup: the restarted owned Alice daemon lacked its intended profile SSH environment; host keys matched. Restarting only that daemon with the correct environment produced native auth exit 0/`authenticated:true`, small history in 0.036 seconds, expected oversized-history `response_too_large` in 0.065 seconds and subsequent small history in 0.031 seconds. This is fixture recovery, not a product fix or an explanation of the historical broken pipe. Live upload/source-change checks retain their separate scope. Manager-backed download pending-confirm/replay now has bounded passes as recorded below; post-confirm replacement, download restart/resume and actual-partial cleanup also pass in bounded live checks; observer slow-reader/fairness and the broader fault matrix remain open. Safe diagnostics are reviewed and committed in `4a2e190b`, with 12 transport and 51 core Crew tests plus strict Clippy/formatting passing; the full local gate/native build now pass on `4a2e190b`; Linux verified production-pair UID 1101 lifecycle validation also passes; the earlier test-feature daemon smoke is excluded. The separate direct probe remains separate-path evidence because it omitted daemon multiplexing/profile arguments.

## Feature and interface parity ledger

The ledger separates committed services, historical interface evidence and remaining runtime gates. Current checks and blockers are summarized above; [plan §15](implementation-plan.md#current-source-milestone-and-unresolved-review) retains the shared contracts and [validation](validation-report.md) retains the chronological evidence.

| ID / feature | Shared service state and owner | GUI state | Native `biorouter crew` state | Completion evidence |
|---|---|---|---|---|
| P01 Profile daemon lifecycle/discovery | Runtime/UDS profile identity, descriptor, lifetime lock and identity/Proven-only stop source present | Shared proxy/detach source present; ASCII proof/Proven attach and stale-descriptor correction source present | Same-socket HTTP identity/peer-UID client and launcher source present | G10/G14: no Electron dependency, concurrent attachment, stale/profile mismatch, bounded stop/restart; Windows shared transport absent |
| P02 Human control/bootstrap | Proven-only digest bootstrap and vault manager/init/unlock/lock with fresh-profile guard source present; vault primitive review closed | Native prompt/confirmation and independently held ASCII human proof source present; no persisted secret or attach-key replacement | Explicit no-echo/secret stdin and digest-only startup source present | G12/G14: proof/refusal and worker negatives, lock/restart/corruption, no secret argv/env/descriptor/settings acquisition |
| P03 Connections, identity and SSH/MFA | Daemon PTY source plus existing manager/broker identity/preflight | Thin Electron WebSocket adapter source present | Three-user SSH and bounded strict two-jump/encrypted-key/PAM MFA pass; full matrix open | G10/G11/G14: terminal MFA with Electron closed, same identities, per-hop trust, echo/cancel/reconnect |
| P04 Teams/channels/invitations/ownership/profile | Existing broker authority and daemon operations | Historical bounded three-client evidence; shared proxy now unvalidated | Three-user enrollment/team/channel and unauthorized-channel refusal passed | G10–G12: matching objects/memberships/ownership and denials |
| P05 History/search/context/read/watch | Shared typed observer and reviewed reconnect correction; focused observer 12 pass | Shared GUI consumer source present; current mixed-interface runtime unqualified | `532c3b7d` passes 60-ID exactly-once backlog, concurrent fairness, replacement slots and bounded membership/derived-source refusal | G10–G12: complete remaining cursor/recovery/queued-derived race and mixed-interface cases; [current evidence](evidence/shared-daemon-532c3b7d-20260923.md) |
| P06 Human posts, references and retries | Existing broker mutation/idempotency and new shared client source | Historical chat/reference evidence; new adapter unvalidated | Three-user posts passed; reference/retry acceptance open | G10–G12: stable retry IDs/receipts and no duplicate/unauthorized publication |
| P07 Upload/download/resume | Shared streaming capabilities/receipts and verified target authority implemented | Native picker/overwrite source present; GUI Save and native Windows remain open | `532c3b7d` three-user file post/read/download completes with matching hashes; earlier 32 MiB restart/reselection/cleanup retains its scope | G10/G11/G13: broader fault/resource/atomic-publication matrix; [current evidence](evidence/shared-daemon-532c3b7d-20260923.md) |
| P08 Owned tasks, approvals and cancellation | Existing task ledger/policy plus new bounded shutdown/cancel-owned-runs source | Historical owned-task/Carol processing evidence; new lifecycle unvalidated | Owned-task cancellation and remote-grant revocation confirmed | G10–G12/G14: same run ownership/IDs, follow/approve/cancel, detach and honest interrupted outcomes |
| P09 Personal-chat grants and privacy | Shared daemon conversation/grant/context/revoke services; standalone local mode separate | Historical GUI consent/API scope remains separate | `532c3b7d` natural `qwen3:8b` Crew result visible once to both peers, provider/source revocation controls and actual PTY leave/abandon/takeover pass | Complete broader privacy/recovery and mixed-GUI cases without worker proof; [current evidence](evidence/shared-daemon-532c3b7d-20260923.md) |
| P10 Shared typed API/client and packaging | `532c3b7d` native/Linux builds and local gate pass; hosted `7ab40c81` has remaining Linux/Windows failures; test-only corrections pass focused checks/full gate; hosted follow-up pending | Local packaged/ad-hoc-signed apps not launched | Unix CLI qualified only for recorded bounded cases; shared Windows CLI unavailable | G15: fix/revalidate hosted failures and complete final-artifact/interface matrix |

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
| G08 | All critical/high ownership, privacy, data-loss and provider-routing defects are fixed and replayed on one final revision. | Independent adversarial review, regression IDs, retest evidence | partial | `aac5f4ac` has all ten local checks, completed daemon/CLI builds, focused route/discovery/navigation tests and all 22 active hosted checks green. The FIOCLEX correction is pushed in `3145dfc5`, passes all ten local checks plus actual ARM64 Linux helper/confinement regression and pinned x86_64 bounded Bullseye smoke; its hosted checks now show 22 successes and one skip, excluding later local commits. Cursor/draft work is committed in `37ac3803` with bounded focused passes; the refreshed Linux artifact passes its recorded contract/helper subset. Native per-hop preflight is committed in `bc3e26fb`, with seven focused and three affected admission/worker passes plus formatting/whitespace checks. Its legacy optional-field case does not establish old-client runtime support. The earlier native/Linux builds, full local gate and final desktop suite pass on product `906bf68b`; verification of the two source-reviewed transfer corrections and fatal-transport correction, broader privacy/provider matrices, transfer restart/replacement and observer backpressure/fairness qualification, new hosted checks and actual graphical workflows remain outstanding; no one-final-revision acceptance is claimed. Retained platform-client omission remains source-reviewed only. |
| G09 | AWS instances, volumes, security groups, keys and fixture credentials are deleted and independently verified. | AWS cleanup report and post-cleanup resource queries | pass | Current fixture cleanup completed at 13:40 UTC: instance terminated, volume/security group/key pair independently absent, local key files removed. The lifecycle JSON records exact identifiers and checks. |
| G10 | Native CLI completes the supported workflow with all Crew Electron clients fully closed. | Process tree, headless profile/auth/MFA, rooms/messages/watch, files/resume, owned tasks, approvals and restart traces | partial | `532c3b7d` passes bounded three-user file/natural-model receipt visibility, 60-ID observer fairness/slots, membership/provider/derived-source controls, actual PTY continuation and native/Linux lifecycle with task-owned Electron closed. [Current acceptance](evidence/shared-daemon-532c3b7d-20260923.md) preserves IDs/hashes; earlier helper/MFA/32 MiB cases retain their original artifacts. Complete broader fault/agent/recovery and final CI/artifact matrix; these passes do not close mixed GUI G11. |
| G11 | Mixed GUI/CLI collaboration among three real Unix users shares identities, connections, messages, transfers and tasks. | Exact IDs/hashes/ownership, bidirectional visibility, policy and context outcomes | not-run | Run after G10 on identified matching artifacts; existing three-GUI evidence remains separate. |
| G12 | Both interfaces preserve Proven-only human authority, owner scope and privacy denial. | Missing/wrong proof, worker human-action attempts, foreign runs, stale grants/cursors, revoked transfer, changed key and zero forbidden sink bytes | partial | Bounded API provider-counter, missing/wrong-proof/no-receipt and foreign-owner cancellation checks pass; complete the broader denial/privacy matrix through both interfaces; daemon API secret, UID, PTY and discovery are not human proof. |
| G13 | Shared daemon transfers preserve bounded memory, binary integrity and local file authority. | Interrupt/restart/resume, no duplicate publication, source/destination replacement, symlink/reparse, unauthorized overwrite and atomic finalization | partial | Earlier review and bounded transfer passes retain their scope. Complete fresh-artifact live replay of the focused-tested overwrite-target and named-partial replacement corrections, then complete the unblocked fault/resource matrix; retain native Save approval separately. |
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

Development apps were locally packaged and separately ad-hoc signed but have not been launched. Native CUA/AWS approvals remain pending; packaging is not GUI acceptance. Final bounded three-user attachment and natural `qwen3:8b` workflow now pass on `532c3b7d`; earlier `5455ebf9` evidence keeps its original scope.

Root verified cleanup of the task-owned Electron clients before current runs; no Crew-fixture Electron process was present at 02:37 UTC. This scopes the current CLI-only runs and does not retroactively upgrade historical evidence.

- The earlier AWS smoke is bounded transport/UID/storage feasibility, not the
  production Crew acceptance test. Its fixture was independently cleaned up.
- The AWS three-account fixture was prepared, but automatic approval
  review rejected private source transfer because specific source-export
  approval was absent. No product source or binary was exported by another
  route. Source-export approval remains absent. This fixture was cleaned up at
  13:40 UTC; a later approved AWS product run requires a fresh fixture.
- The current local Docker fixture runs Ubuntu 24.04.5 LTS (`ubuntu:24.04`)
  on LinuxKit `6.12.76` aarch64 with real accounts `alice=1101`, `bob=1102`,
  `carol=1103`. Broker SHA-256
  `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`
  is installed in every account's `~/.local/bin/biorouter-crew` and at the
  fixture's `/usr/local/bin/biorouter-crew`. Connecting clients use the
  per-account path. Older Debian/runtime/port observations are historical
  evidence, not the current fixture configuration; no current port is asserted.
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
