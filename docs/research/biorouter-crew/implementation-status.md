# BioRouter Crew implementation and acceptance status

Status is recorded against the final revision under test. A lower-level fixture
or unit test never upgrades a real-app, cross-platform, or institutional gate.
Each row must have an evidence path before it can be marked `pass`.

Legend: `not-run` = no execution yet; `blocked` = an external prerequisite is
missing; `fail` = the observed workflow failed; `partial` = only part of the
required evidence exists; `pass` = the stated behavior was observed on the
recorded revision. A later change requires the affected checks to be repeated.

Published source `e53edfb1a485b74734decba149969c773b2e1ef4` is on draft PR #366. The current institution revision requires an institution ID for new private SSH connections and an explicit, immutable workspace label confirmed by its host. Local models can serve any institution; institutional models must match the workspace. Public/private toggles retain the label. Legacy unlabelled private workspaces permit human collaboration but cannot grant agent access until labelled. Grants bind connection/workspace policy epochs, resolved provider affiliation and authoritative protected-source state; a protection change invalidates an earlier unprotected grant before worker output. Institution provenance remains mandatory when optional privacy settings are off.

All Crew-scoped copy, diverge and edit-diverge operations are refused before creating a child, including expired or revoked scopes. Start a fresh conversation and explicitly grant its Crew context. Ordinary conversation derivation retains the institution-owner union. This is an explicit workflow limit, not completed derivation parity.

Current source validation passes: 62 core Crew tests, 6 session-provenance tests, 36 broker integration tests (4 adversarial, 28 broker and 4 cursor), 37 GUI tests, typecheck and independent Astra review. The final `just check-everything` gate exited 0 at 23:12:36 UTC on 2026-09-23 after regenerating the staged API contract. The journal target selected zero tests and supplies no evidence. Hosted checks last verified at `a3ec1924` were all green; they do not qualify this institution revision. The clean-source native CLI/daemon build is now verified; all four supported daemons are upgraded and their automatic-stdin GUI clients are renderer-ready after normal reload. This is installed-artifact/launch proof only; supported real-Versa self-test and live institution acceptance remain pending. Official Linux backend workflow 35932706989 succeeds and its artifacts are verified. Required e53 CI has two stale-fixture failures (test-module census and observer epoch/label); local corrections pass 1+1+6 selected tests. A subsequent resume checkpoint passed the full gate at 00:26:01 UTC, but it predates the latest decision-helper and observer changes. The factory suite passes 18 tests, including the marked-binding regression and the privacy-order audit passes one; the helper behavioral test now passes, factory tests pass 18, and the final full gate exits 0 at 00:41:39 UTC. All product diffs are independently reviewed clean; native rebuild and runtime replay remain pending.

Prior installed runtime evidence uses backend source `533d4a7c` and the separately identified renderer. The `4339916c` send correction passed an actual Alice-to-Carol Enter submission without the join-screen flicker, and Shift-Enter inserted a newline. All three GUI clients used private Versa `gpt-5.5-2026-04-24` to read and attach their preexisting totals; these were recovery tasks, not new calculations. Earlier model failures, non-blind setup context, native Save passes and the latest inconclusive Save attempts retain their separate artifact scopes in [validation](validation-report.md). The actual Versa GPT-5.5 self-test has a typed no-grant refusal pass, but child help commands used stale PATH/repository binaries and do not qualify e53. Correct local resume then exposed a saved-provider marker rejection. The trusted session-store restoration fix passes 18 factory tests, including the marked-binding regression; full self-test acceptance remains incomplete.

Earlier AWS fixtures remain independently cleaned. A new e53 fixture now has four ordinary UIDs 10001–10004 and two rootless brokers; its cleanup deadline is 2026-09-24 01:10:57 UTC and cleanup is pending. A bounded GUI institution negative now passes: the fourth profile connected, initialized and confirmed foreign-synthetic/private, then its UCSF-affiliated private Versa task was refused for institution mismatch. The CONNECT recorder stayed zero bytes before/after, independently checked. No run/context receipt was visible. A reviewed observer starvation correction resets last_state after successful delivery-time state reauthorization; source checks and final full gate pass; the new live fairness case is ignored/unexecuted and runtime replay remains pending. Latest observer requests use initial:latest. Independent CLI no-run verification was blocked by automatic review; specific user approval was requested and no CLI pass is claimed. Alice’s normal GUI connect/init/ucsf confirmation and Bob/Carol connection, enrollment and team-invitation acceptance pass. All three members and persisted messages are visible in E53UCSFAcceptanceTeam/#general. Separately, Carol’s Enter marker appeared automatically in untouched Bob view after 4,073 ms without refresh, reconnect or join flicker; Bob’s Shift-Enter produced the exact two-line composer. Fresh Versa tasks now complete for all three owners through normal UI with remote execute exit 0, read/write/attachment and expected row counts/totals. Earlier attachment IDs in Bob/Carol agent-session history did not qualify channel rendering. Alice subsequently used normal Reconnect, caught up to 117 channel messages and rendered all three attachment cards: the earlier absence was delayed projection, not lost records. Alice’s actual native Save of Carol’s 55-byte attachment matches its independently verified remote hash. Initial catch-up took approximately 75–90 seconds. This bounded e53 peer-visibility/Save pass does not qualify the newer fairness correction, which is not installed. Alice’s ordinary Versa chat now passes normal /crew consent, Connections and context.manifest retrieval with all three human markers and the exact selected workspace; posting/revocation is separate and pending. The generic observer alert has no decoded code and is not proven stale_cursor. Alice’s independent SSH receipt proves file UID/size/hash only; Bob/Carol independent SSH corroboration now proves file UID/size/hash, completing that file-evidence subset for all three owners; it does not independently prove job exit.

Remaining acceptance covers installed-artifact verification and live institution boundaries, the coherent GUI/CLI file and owned-agent workflow, ordinary-chat MCP and selected-channel context, required self-test assertions, and affected recovery/privacy checks. The broader platform/fault matrix remains partial. Shared native CLI IPC is Unix-only; Windows desktop acceptance is incomplete. Leo execution is unsupported on its recorded Landlock ABI 1, and Narrows NFS broker hosting is unsupported. Synthetic evidence does not establish institutional deployment or HIPAA compliance. Screenshots, secrets and raw visual/transcript evidence remain local.

### Artifact provenance

| Artifact / evidence role | Source | SHA-256 / qualification |
|---|---|---|
| Current verified Linux build | `e53edfb1`, backend workflow 35932706989 | CLI `b3627f82dbcdb2672882ae91b18c21c190970080cace9872580126ef2381564c`; daemon `b93f26a4d2bcdcec0b291b6d523c66eae24207d4c18098abceed28ff5adca4d5`; broker `ab0a4532a83166be7430b23ba472f79ebf49008556debb98bb03b97baf099319`. ELF x86-64 mode 0555, maximum GLIBC 2.30; live institution workflow pending. |
| Current native CLI build | Clean `e53edfb1`, institution native manifest | `70cb5f24d584218e44b16283827edcbb02359911d1aaba9dc464190140aa8f02`; 1.91.1, mode 0555; build/version/help verified, live acceptance pending. |
| Current native daemon build | Same e53 manifest | `e710f908f7fa055f6600c4a3a4345b0dd6c37ef6650f974956ca25165b453065`; 1.91.1, mode 0555; build and four-profile installed launch verified; live institution acceptance pending. |
| Prior verified Linux x86-64 artifacts | `533d4a7c`, workflow 35911556727; `/private/tmp/crew-linux-533d4a7c-ci/verified/VERIFIED-MANIFEST.json` | CLI `bdeccf522d710f58f5cb71f32158964c8bbc289b65be007c41356633d2548c20`; daemon `9dd2c6e57b4076fc4a56f23e5904bc9c7bcd3cae8866aaca959237f550c24f8d`; unchanged broker `edc3133a8d80489b26497374d32472690179247b33394f0423ef7955fd81aac4`. Exact public artifacts hash-verified for three users; bounded fresh Alice daemon lifecycle/wrong-proof/restart/stop passes. No full Linux workflow or GUI qualification. |
| Prior native CLI | `533d4a7c`, memory artifact manifest | `015a11e7b32ffd717dc212c63e11dc9d9631b4b0c19dd2173892babd9e8169b8`; macOS arm64 dev 1.91.1, mode 0555; build/current bounded native checks pass, final GUI pending. |
| Prior native daemon | Same 533 artifact and completed upgrade manifest | `979c181c7ddc605d2c541eadb949a64bc9493b51232d9e33596a0bf5b014990c`; installed for all three profiles, no final GUI qualification. |
| Prior native CLI | Deadline pair, `/private/tmp/crew-deadline-native-artifacts-20260923T190049Z` | `f216e05b9d918f0bb50ac19cb632bb250b02774fdfea2ec5bff4fbd89647c04a`; full local gate/build pass; final GUI/runtime matrix pending. |
| Prior native daemon | Same deadline pair and handoff manifest | `4af17abe9d233b559a368ab7a617a1f2ae6b0dd8064fcf01309046f6a059c6bf`; installed for all three stable profiles; Alice attach timed out, Bob/Carol not launched. |
| Prior native CLI build | Bodyless-header pair, `/private/tmp/crew-observer-serialization-artifacts-20260923T000000Z` | `5ee21a9547e5e52b0248108370d3d7d746e28b85d94e8c4ba02fde45b75724e8`; full gate/build pass. The unchanged daemon is `0890a909cd1e6933ae6aa06549275e5d0152a72c006b21a7bb713de0c7ee2cf6`; broader earlier runtime evidence retains its source scope. |
| Prior native daemon used with 6c checkpoint | Compression pair, `/private/tmp/crew-gui-pair-20260923T0842Z` | `0890a909cd1e6933ae6aa06549275e5d0152a72c006b21a7bb713de0c7ee2cf6`; actual Alice observer/rendering and named Save evidence; not a rebuild of later source. |
| Prior validated native CLI | `7ab40c81` | `ffa2f599621159be35a9efbf9c6f74c0ce5f3497d72da41d0d6bcece29c09624`; ordinary native build and supported SSH/exact Unicode-history smoke pass; broad `532c3b7d` end-to-end results are not reassigned. |
| Prior runtime-qualified native CLI | `532c3b7d` | `edc9a59ba6afbc37df6b19d09070b3c931f4bd2a6de91f5f3c7965bd165efdef`; non-test debug arm64 Mach-O 1.91.1, mode 0555; full gate/build and bounded PTY/observer/membership/provider/derived-source acceptance pass; final bounded three-user file and natural `qwen3:8b` replay pass; broader matrix remains open. |
| Prior validated native daemon | `7ab40c81` / identical `532c3b7d` | `4a1082ec9e1e0cc0b464c8cd608156a21cd24544c27cd87a2d42e7f84b16185c`; non-test debug arm64 Mach-O 1.91.1, mode 0555; same-source Linux build/lifecycle pass, broader platform matrix open. |
| Prior validated native CLI | `5455ebf9` | `b329b6ad161e9b6722ef6b08ebef9aab3df4462c25c516a3206f051525fc7bdc`; arm64 Mach-O 1.91.1, mode 0555; full gate/release build, bounded deterministic tool/revocation and natural `qwen3:8b` Crew receipt acceptance pass, with independent Bob/Carol visibility; broader cases and later-fix replay remain pending. |
| Prior validated native daemon | `5455ebf9` | `e3bcbf581b4ec06f4a24e7a76a27f2843c36e9ef3e8b100087ab85021d5d5498`; arm64 Mach-O 1.91.1, mode 0555; merged `3dac3695` not covered. |
| Prior validated Linux ARM64 CLI | `dd70051e` | `381eb225ab82015e85d1552f9629f887f206553e3fe017dd8e8c8cc85bcc1f67`; ordinary non-test ARM64 build, GLIBC 2.39, mode 0555; fresh UID 1101 lifecycle and wrong-proof refusal pass. |
| Prior validated Linux ARM64 daemon | `dd70051e` | `81b7a2e3e4f62161490f04535bf11308c3c03aa96b305f27b2cf35d8a9ce8318`; byte-identical to recorded 532 Linux daemon; fresh UID 1101 lifecycle passes. |
| Prior validated Linux ARM64 CLI | `532c3b7d` | `35c52efd390166f59a48325d93211b6580feaf14efb9d129e162c0a900af2d73`; non-test debug build, GLIBC 2.39, mode 0555; focused suites and UID 1101 lifecycle pass. |
| Prior validated Linux ARM64 daemon | `532c3b7d` | `81b7a2e3e4f62161490f04535bf11308c3c03aa96b305f27b2cf35d8a9ce8318`; non-test debug build, GLIBC 2.39, mode 0555; same-profile/new-instance and wrong-proof refusal pass. |
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
| Historical three-user Linux ARM64 broker | Cursor source subsequently committed in `37ac3803`; build provenance in validation report | `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`; 30 passed, one ignored plus exact helper/confinement regression. At that checkpoint installed through all three ordinary users with preserved state; bounded three-user CLI collaboration, file, context and personal MCP results are recorded below; full current-artifact acceptance remains open. |
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

Canonical institution/provider binding is required implementation work in progress, not deferred roadmap scope. Other broader plan capabilities are absent or unsupported: NFS/SMB
canonical storage,
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

The ledger separates committed services, historical interface evidence and remaining runtime gates. Current checks and blockers are summarized above; [plan §15](implementation-plan.md#implemented-services-and-remaining-acceptance) retains the shared contracts and [validation](validation-report.md) retains the chronological evidence.

| ID / feature | Shared service state and owner | GUI state | Native `biorouter crew` state | Completion evidence |
|---|---|---|---|---|
| P01 Profile daemon lifecycle/discovery | Runtime/UDS profile identity, descriptor, lifetime lock and identity/Proven-only stop source present | Shared proxy/detach source present; ASCII proof/Proven attach and stale-descriptor correction source present | Same-socket HTTP identity/peer-UID client and launcher source present | G10/G14: no Electron dependency, concurrent attachment, stale/profile mismatch, bounded stop/restart; Windows shared transport absent |
| P02 Human control/bootstrap | Proven-only digest bootstrap and vault manager/init/unlock/lock with fresh-profile guard source present; vault primitive review closed | Native prompt/confirmation and independently held ASCII human proof source present; no persisted secret or attach-key replacement | Explicit no-echo/secret stdin and digest-only startup source present | G12/G14: proof/refusal and worker negatives, lock/restart/corruption, no secret argv/env/descriptor/settings acquisition |
| P03 Connections, identity and SSH/MFA | Daemon PTY source plus existing manager/broker identity/preflight | Thin Electron WebSocket adapter source present | Three-user SSH and bounded strict two-jump/encrypted-key/PAM MFA pass; full matrix open | G10/G11/G14: terminal MFA with Electron closed, same identities, per-hop trust, echo/cancel/reconnect |
| P04 Teams/channels/invitations/ownership/profile | Existing broker authority and daemon operations | Shared proxy has bounded three-client messaging, invitations and identity evidence; final 533 GUI unqualified | Three-user enrollment/team/channel and unauthorized-channel refusal passed | G10–G12: matching objects/memberships/ownership and denials |
| P05 History/search/context/read/watch | Shared typed observer and reviewed reconnect correction; focused observer 12 pass | Shared GUI consumer source present; current mixed-interface runtime unqualified | `532c3b7d` passes 60-ID exactly-once backlog, concurrent fairness, replacement slots and bounded membership/derived-source refusal | G10–G12: complete remaining cursor/recovery/queued-derived race and mixed-interface cases; [recorded evidence](evidence/shared-daemon-532c3b7d-20260923.md) |
| P06 Human posts, references and retries | Existing broker mutation/idempotency and new shared client source | Bounded shared-adapter GUI/CLI messaging passes; final retry matrix open | Three-user posts passed; reference/retry acceptance open | G10–G12: stable retry IDs/receipts and no duplicate/unauthorized publication |
| P07 Upload/download/resume | Shared streaming capabilities/receipts and verified target authority implemented | Bounded PNG/binary native Save and receipt cleanup pass; broader Save/native Windows open | `532c3b7d` three-user file post/read/download completes with matching hashes; earlier 32 MiB restart/reselection/cleanup retains its scope | G10/G11/G13: broader fault/resource/atomic-publication matrix; [recorded evidence](evidence/shared-daemon-532c3b7d-20260923.md) |
| P08 Owned tasks, approvals and cancellation | Existing task ledger/policy plus new bounded shutdown/cancel-owned-runs source | Historical owned-task/Carol processing evidence; new lifecycle unvalidated | Owned-task cancellation and remote-grant revocation confirmed | G10–G12/G14: same run ownership/IDs, follow/approve/cancel, detach and honest interrupted outcomes |
| P09 Personal-chat grants and privacy | Shared daemon conversation/grant/context/revoke services; standalone local mode separate | Historical GUI consent/API scope remains separate | `532c3b7d` natural `qwen3:8b` Crew result visible once to both peers, provider/source revocation controls and actual PTY leave/abandon/takeover pass | Complete broader privacy/recovery and mixed-GUI cases without worker proof; [recorded evidence](evidence/shared-daemon-532c3b7d-20260923.md) |
| P10 Shared typed API/client and packaging | `532c3b7d` native/Linux builds and local gate pass; `dd70051e` passes hosted Rust on all three platforms and the frontend workflow after test-only portability corrections | Historical packaged-shell refusal; supported GUI subsets pass, final 533 GUI readiness pending | Unix CLI qualified only for recorded bounded cases; shared Windows CLI unavailable | G15: complete final-artifact/interface matrix and verify subsequent published heads |

[Plan §15](implementation-plan.md#implemented-services-and-remaining-acceptance) records accepted contracts and open review. Shared-adapter GUI evidence qualifies only its recorded subsets and artifacts; it does not close full native CLI parity.

## Release gates

| ID | Gate and acceptance invariant | Type/evidence required | Status | Owner / next action |
|---|---|---|---|---|
| G01 | Actual BioRouter Electron development app is built from this worktree and three independent app clients drive the visible Crew UI. | Build command, revision, sanitized screenshots/video, action timeline | pass at recorded revision | Current e53 native artifacts and institution renderer are installed for four profiles with automatic launch/reload proof. Three collaborating clients have normal enrollment/messaging and fresh Versa task evidence. Alice’s e53 channel now renders all three attachment cards and native peer Save passes; earlier graphical results retain their artifact scopes in validation. |
| G02 | Three real Unix accounts use distinct UIDs, home directories, SSH identities and profiles; no shared key or copied provider credential. | Disposable AWS fixture report with account/key/UID mapping | partial; recorded AWS three-user setup passes | Active e53 AWS fixture has four ordinary UIDs 10001–10004, isolated identities and two rootless brokers. Three collaborating members completed normal UI enrollment/team acceptance; the fourth hosts the foreign-institution case. Its cleanup deadline is 2026-09-24 01:10:57 UTC. |
| G03 | Alice hosts the broker from a user-writable home; product install/start/restart/recovery is rootless and creates no system account/group/service/global SSH change. | Privilege trace, filesystem manifest, process UID, install/recovery transcript | partial | At the historical checkpoint, the `a38b34be…` broker was installed through each ordinary UID and restarted as Alice (1101), preserving workspace/key/node/socket and journal/history. The pinned x86_64 `eefaece1…` passes bounded service smoke as UID 65532. Earlier wrong-node/torn-tail, quotas and injected journal write/fsync failures retain their own evidence. Power loss and the full storage-fault matrix remain unexecuted; NFS is unsupported. |
| G04 | Alice, Bob and Carol complete connect/enroll/general/analysis/files/owned-agent/context/revocation/recovery workflow with human chat before model selection. | Three-app evidence bundle and event IDs | partial | Fresh e53 Versa runs complete remote execute exit0/read/write/attach for Alice/Bob/Carol. All three file UIDs/bytes/hashes are independently verified, separate from job-exit metadata. Alice’s normal Reconnect caught up to117messages/all3cards, followed by native peerSave/hash pass; initial projection delay remains recorded. Alice Connect returned 200 with verified connected identity, but its observer alert has no decoded code. The reviewed starvation fix resets last_state after successful delivery-time state reauthorization; source checks pass; rebuild/runtime replay remain pending; ordinary-chat MCP read/context passes through normal GUI consent; separate posting/revocation remains pending. |
| G05 | Real provider positive pass uses the actual BioRouter provider path and a public-safe fixture; negative pass uses a designated synthetic public sink and observes zero forbidden canary bytes. | Provider identity, sink digest/bytes, dispatch/tool/context traces | partial | Epoch 3 GUI public assistant control and fresh Private Crew 400/no extra dispatch pass; current 533 identical-ID native refusal replay also preserves sink count. Earlier sink epochs retain their invalid/provisional limits. Complete the remaining graphical personal-MCP, source-policy and final-artifact matrix. |
| G06 | 10, 30 and 50 independently authenticated Unix participants exercise bounded load; the 50-user target is a 30-minute soak with three graphical clients retained. | Workload seed, p50/p95/p99, CPU/RSS/open files, journal/replay, UI responsiveness | partial | The first standalone 50-user run observed 1,500 acknowledged/read-back messages but lacked complete duration/restart/resource evidence. The second `7311f126…` run records a top-level 30-minute workload, 1,500 exact IDs/body hashes, zero errors/disconnects and exact same-state restart replay. Latency p50/p95/p99 was 172.94/712.79/903.92 ms; broker peak RSS was 1,262,800 KiB. Per-participant duration timestamps were absent. Its broker process/workspace was separate from the GUI fixture, even though both used the same binary artifact; this does not prove GUI responsiveness under load. The historical `4f11d858…` 30-minute/50-UID run records 1,500/1,500 acknowledgments and exact readback/restart with zero loss/hash mismatch/errors; [curated evidence](evidence/crew-50-uid-soak-20260922.md) is available; combined-interface/resource qualification remains open. |
| G07 | macOS, Windows and Linux client compatibility is measured separately for OpenSSH/MFA/process lifecycle. | Per-OS run records; no macOS substitution for other OSes | partial | Native CLI fixtures now cover forwarding-disabled approved exec, SFTP-only refusal, changed gateway/final-target keys and two manual PAM reconnects. These used no Crew artifact. The earlier short-path fix passed Electron PAM authentication/explicit close on `1ffa69d9…`, not a complete PAM Crew workspace connection. Desktop policy/reconnect cases, institutional MFA and independent Windows/Linux desktop acceptance remain open. New shared IPC is Unix-only. The Windows helper is fully integrated in source; native runtime/durability acceptance remain open; required Windows desktop transfer functionality is not yet qualified. |
| G08 | All critical/high ownership, privacy, data-loss and provider-routing defects are fixed and replayed on one final revision. | Independent adversarial review, regression IDs, retest evidence | partial | Bounded transfer target/partial/replay/restart/cleanup, transport recovery and `532c3b7d` observer fairness/slots now have recorded passes; [validation](validation-report.md) preserves each source/artifact scope. Complete remaining privacy/provider and fault/resource matrices, native platform and actual graphical workflows on identified final artifacts. Existing passes do not establish one-final-revision acceptance or explain historical failure causes. |
| G09 | AWS instances, volumes, security groups, keys and fixture credentials are deleted and independently verified. | AWS cleanup report and post-cleanup resource queries | prior cleanup passes; current pending | Prior fixtures are independently cleaned. Current four-account/two-workspace e53 fixture remains active with cleanup deadline 2026-09-24 01:10:57 UTC; its separate teardown verification is pending. |
| G10 | Native CLI completes the supported workflow with all Crew Electron clients fully closed. | Process tree, headless profile/auth/MFA, rooms/messages/watch, files/resume, owned tasks, approvals and restart traces | partial | `532c3b7d` passes bounded three-user file/natural-model receipt visibility, 60-ID observer fairness/slots, membership/provider/derived-source controls, actual PTY continuation and native/Linux lifecycle with task-owned Electron closed. [Recorded acceptance](evidence/shared-daemon-532c3b7d-20260923.md) preserves IDs/hashes; earlier helper/MFA/32 MiB cases retain their original artifacts. Complete broader fault/agent/recovery and final CI/artifact matrix; these passes do not close mixed GUI G11. |
| G11 | Mixed GUI/CLI collaboration among three real Unix users shares identities, connections, messages, transfers and tasks. | Exact IDs/hashes/ownership, bidirectional visibility, policy and context outcomes | partial | Alice app/CLI instance identity matches and bidirectional message visibility passes with exact IDs/body. Bob and Carol GUI posts render across all three clients with independent CLI ID/actor/channel matching; shared file/task/privacy and detach/reopen acceptance remain pending. Ordinary Versa chat also passes normal /crew consent and read-only Connections/context.manifest retrieval of three markers and exact workspace; posting/revocation remains pending. |
| G12 | Both interfaces preserve Proven-only human authority, owner scope and privacy denial. | Missing/wrong proof, worker human-action attempts, foreign runs, stale grants/cursors, revoked transfer, changed key and zero forbidden sink bytes | partial | Bounded API provider-counter, missing/wrong-proof/no-receipt and foreign-owner cancellation checks pass; complete the broader denial/privacy matrix through both interfaces; daemon API secret, UID, PTY and discovery are not human proof. |
| G13 | Shared daemon transfers preserve bounded memory, binary integrity and local file authority. | Interrupt/restart/resume, no duplicate publication, source/destination replacement, symlink/reparse, unauthorized overwrite and atomic finalization | partial | Bounded target-change refusal, pending-confirm/replay, post-confirm replacement, 32 MiB restart/resume and actual-partial cleanup pass at their recorded revisions; `532c3b7d` also passes three-user file visibility/download hashes. Complete broader memory/resource, storage-fault, reparse/platform and queued-revocation cases; actual native Save and mixed-GUI acceptance remain separate pending gates. |
| G14 | Authentication and shared-daemon/controller lifetime preserve secrets and honest recovery. | Prompt echo/input/resize/cancel, controller exclusivity, GUI/CLI detach, restart, exact-process shutdown and secret sweep | partial | Isolated approval/vault/stop/new-instance locked restart passed; strict two-jump/encrypted-key/PAM MFA, wrong-secret/cancel, retained connection after CLI exit and secret sweeps pass; wider controller/recovery matrix remains open. Validate corrected GUI reopening, exact-ID cleanup, bounded shutdown and shared lifecycle/auth; no weaker gate or model-visible controller credential. |
| G15 | One final revision delivers shared typed services and complete parity evidence. | CLI commands/help, generated schemas, exact binaries/commit, required checks, independent reviews and G10–G14 traces | not-run | Published e53 source full gate and native/Linux builds pass. Required CI has two stale-fixture failures with local selected-test corrections; resume regressions and an earlier full gate pass; latest decision-helper/observer changes now pass their focused checks and final full gate; rebuilt runtime replay remains pending. Live subsets pass, including bounded peer attachment visibility/native Save, but remaining institution/recovery checks and required self-test assertions do not yet qualify one final revision. |

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

The packaged-shell guard refusal and earlier three-client GUI successes retain their recorded scopes. Current 533 native upgrade is complete, but final GUI readiness/reopen remains unqualified. Earlier PNG/binary Save, three-user messaging and bounded provider/ownership refusals are passes, not a complete mixed-interface workflow.

Root verified cleanup of the task-owned Electron clients before current runs; no Crew-fixture Electron process was present at 02:37 UTC. This scopes those earlier CLI-only runs and does not retroactively upgrade historical evidence.

- The earlier AWS smoke is bounded transport/UID/storage feasibility, not the
  production Crew acceptance test. Its fixture was independently cleaned up.
- The AWS three-account fixture was prepared, but automatic approval
  review rejected private source transfer because specific source-export
  approval was absent. No product source or binary was exported by another
  route. That rejection remains historical. The fixture was cleaned up at
  13:40 UTC. Verified-binary and synthetic-data transfers to a fresh disposable
  AWS fixture are now explicitly approved and verified binary installation/CLI
  lifecycle have passed; full collaboration remains pending, without a broader
  source-export claim.
- The current local Docker fixture runs Ubuntu 24.04.5 LTS (`ubuntu:24.04`)
  on LinuxKit `6.12.76` aarch64 with real accounts `alice=1101`, `bob=1102`,
  `carol=1103`. Broker SHA-256
  `4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`
  is installed in every account's `~/.local/bin/biorouter-crew` and at the
  fixture's `/usr/local/bin/biorouter-crew`. Connecting clients use the
  per-account path. Older Debian/runtime/port observations are historical
  evidence, not the current fixture configuration; no current port is asserted.
- Current daemon/CLI and new Linux broker provenance are listed above. Hosted
  green is established on exact `533d4a7c` for all required PR checks; its Linux package workflow also succeeds. Historical artifacts retain separate provenance.
  Cursor/draft and SSH-preflight focused evidence is bounded as recorded above;
  final combined acceptance is still open. Exact historical checks and model failures remain in
  the linked reports.
- Native Save now has a bounded actual named-file pass: 75-byte PNG, mode 0600,
  exact source/save hash. Broader Save/platform and recovery cases remain open; bounded manual and upload cleanup pass. Carol's task-local `qwen3:8b` processing/attachment has a rendered
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

### Institution acceptance preparation

The fourth profile’s normal Settings/refusing-proxy control is established, and the live foreign-institution GUI refusal keeps its recorder at zero. Independent CLI no-run corroboration is blocked pending specific approval. The active e53 AWS fixture has four ordinary users and two workspaces, with cleanup due 2026-09-24 01:10:57 UTC. Prior fixtures are independently cleaned; this fixture requires its own cleanup receipt. Fresh three-owner Versa execution/read/write/attachment passes are scoped in validation, including Alice’s later all-three-card visibility and Carol-file native Save; ordinary-chat MCP read/context now passes; separate posting/revocation and fairness-fix replay remain pending.
