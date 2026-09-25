# BioRouter Crew implementation and evidence

Start with the [comprehensive implementation plan](implementation-plan.md), revised September 22 after the user's decisions. **Section 15 makes daemon-owned native `biorouter crew`/GUI functional parity a completion requirement**, including terminal MFA/human approvals, shared transfers/tasks, CLI-only acceptance with Crew Electron clients fully closed, then mixed GUI/CLI collaboration among three real Unix users. Earlier GUI-first progress does not measure this scope. Sections 1, 3, 6, 9 and 13 define no-admin home-based deployment, 2–50-user labs, the cluster Public/Private toggle, shared MCP/SSH capabilities and real three-user dev-app acceptance testing. It incorporates the requested SSH/MFA/jump-host support, simple Linux/text storage preference, human and owned-agent collaboration, files, teams/channels and mandatory private/public boundaries.

Work resumed on 2026-09-23 with two more acceptance requirements, recorded in [plan §16](implementation-plan.md#16-resumed-scope-gui-redesign-and-human-readable-identity-2026-09-23): a Slack-like desktop redesign ([UI redesign specification](ui-redesign-spec.md)) and people, teams and channels addressed by name instead of machine ID, with joining by invitation and device code ([naming design](naming-design.md)). The [resume handoff](handoff-2026-09-24.md) carries live package status and the dated decisions log; the [status ledger](implementation-status.md) records what has been measured.

Close-out, 2026-09-25: every §16 package is implemented, and acceptance ran as four live QA rounds with fresh novice critics (unaided task rate 39% → 57% → 85% → 91%) and seven evidence-grade lanes on the final build `461f7899`. The [redesign acceptance evidence](evidence/ui-redesign-acceptance-2026-09.md) records what passed, the one failed assertion and what remains unverified; the [handoff's close-out section](handoff-2026-09-24.md#close-out-handoff-2026-09-25) lists what is deferred and the security-sensitive commits that need human review before merge.

| Artifact | Contents |
|---|---|
| [Implementation plan](implementation-plan.md) | Architecture, data/protocol/storage contracts, permissions, UI, deployment, phased implementation and acceptance gates |
| [Acceptance status](implementation-status.md) | Feature-by-service/interface parity ledger, G01–G15 release gates, remaining blockers and artifact provenance |
| [Resume handoff](handoff-2026-09-24.md) | The close-out handoff (what is done, final artifacts, gates, deferred items, human-review list, how to resume), then the campaign record (work-package progress, workstreams, decisions log) above the 2026-09-24 pause receipts it supersedes in part |
| [Redesign acceptance evidence](evidence/ui-redesign-acceptance-2026-09.md) | The redesign and naming campaign's four live QA rounds, per-round security results, the seven final acceptance lanes on `461f7899` with IDs and hashes, fixture provenance and what remains unverified |
| [Naming design](naming-design.md) | How people, workspaces, teams and channels are named instead of numbered: display rules, name keys, uniqueness, the daemon resolver, joining by invitation and device code; decisions D1–D17, slices S0–S4 |
| [UI redesign specification](ui-redesign-spec.md) | The Slack-like Crew desktop GUI: layout, every screen mapped from the old UI, component architecture, copy deck, identity display, revoke, privacy, motion, accessibility and test migration |
| [Broker protocol](protocol-contract.md) | Wire contract of the remote broker: transport and identity, `hello` v1 and v2, collaboration methods, names, joining by invitation and device code, attachments, owned-agent grants, durability and rootless setup |
| [Native CLI source plan](cli-parity-source-plan.md) | Source inventory and proposed command/client/lifecycle seams; proposals alongside accepted bootstrap/credential design; implementation and parity acceptance remain open |
| [Native CLI guide](cli-guide.md) | Current commands, shared desktop daemon, explicit approvals, vault, SSH/MFA, collaboration, files, agents and recovery |
| [Validation report](validation-report.md) | Exact build/check commands, selected test counts and artifact hashes |
| [Three-client app evidence](crew-ui-acceptance-report.md) | Actual isolated Electron clients, human collaboration, files, MFA and agent workflow observations |
| [Provider boundary evidence](local-provider-boundary-report.md) | Public positive control and private synthetic canary refusals |
| [Adversarial validation](adversarial-validation.md) | Ownership, revoked uploads, writer locking and journal faults |
| [50-user soak](local-real-uid-50-soak-report.md) | Actual Unix identities, load metrics and restart qualifications |
| [Linux portability](linux-portability.md) | Pinned glibc floor, broker packaging, rootless installation and artifact qualification |
| [Invariant coverage](regression-coverage-map.md) | I01–I24 evidence plus P01–P10 parity regressions; explicit interface and acceptance gaps |
| [Platform research](platform-research.md) | Original five options plus Matrix, Zulip, Mattermost, Rocket.Chat, Tinode and NATS/native; primary sources, licenses and limitations |
| [Current architecture/privacy](architecture-privacy.md) | Verified BioRouter source paths/lines; reusable extension/agent/session seams and security gaps |
| [SSH hop policy](ssh-hop-policy.md) | Per-hop native configuration admission, host-trust and multiplexing requirements, and client limitations |
| [Current SSH/UI architecture](ssh-ui-architecture.md) | Actual connection, navigation, file and stream support; new work and MFA design |
| [Feasibility report](feasibility-report.md) | Measured results on Narrows, Leo and one disposable AWS fixture; explicit untested boundaries |
| [Institutional probe evidence](institutional-host-smoke.json) | Machine-readable final results and 11 primitive checks per host |
| [AWS probe report](aws-identity-smoke-report.md) | Two real Unix accounts, simulated jump route, binary transfer, history replay and verified teardown |
| [Probe scripts](smoke/) | Small reproducible synthetic tests; not production Crew code |
| [Privacy review](review-privacy.md) and [SSH review](review-ssh.md) | Independent review findings and resolution records |
| [Broker source review](independent-review-broker.md) | 2026-09-22 independent review of the broker and its lifecycle: findings and the corrected source for each |
| [Transport, server and UI source review](independent-review-transport-server-ui.md) | 2026-09-22 independent review of the SSH transport, daemon routes and desktop UI: findings and recheck state |
| [Institutional SSH compatibility](institutional-ssh-compatibility.md) | Read-only metadata and kernel-capability probes of the two institutional SSH targets, 2026-09-22 |
| [Linux SSH PAM probe](linux-ssh-pam-test-report.md) | Synthetic keyboard-interactive PAM probe against a disposable container; not production MFA |
| [Local Linux SSH fixture](local-linux-ssh-fixture-report.md) | The localhost Docker fixture with three synthetic Unix users used by earlier Crew QA |
| [Local multi-hop SSH probe](local-multihop-ssh-test-report.md) | Four localhost `sshd` listeners with distinct host keys: `ProxyJump`, cancellation and changed-key refusals |
| [Local Linux CLI and helper canary](local-linux-cli-canary-report.md) | Linux ARM64 broker, journal and peer-credential canaries on a local container |
| [Desktop integration handoff](desktop-integration-handoff.md) | The build and launch contract for the three-client desktop runs; not evidence that a launch passed |
| [QA fixture integrity](qa-fixture-integrity.md) | Why the first Bob enrollment attempt is excluded: the fixture's saved connection was altered by hand |
| [Evidence records](evidence/README.md) | Curated evidence summaries for bounded live runs, each with its source revision and scope |

Earlier `5455ebf9` passes the full local gate, native/Linux builds and bounded shared-CLI deterministic and natural `qwen3:8b` Crew-tool acceptance. Merged checkpoint `3dac3695` is followed by API/docs `8be945c2`; the merged UI passes 562 files/6,366 tests/19 skips, typecheck and 60 affected tests. See [current status](implementation-status.md).

Runtime-qualified source `532c3b7d` commits reviewed native continuation recovery and observer backlog corrections. Valid fresh-process CLI 11, daemon-client 16 and observer 12 regressions pass, and the full gate/native non-test build pass. Linux build/lifecycle, actual PTY leave/abandon/takeover and bounded 60-ID backlog/fairness/slot replay now pass. Published baseline `7ab40c81` has a passing native build and bounded SSH/Unicode smoke; its daemon is byte-identical to `532c3b7d`. Test-only portability corrections in `dd70051e` pass integration 42/42, core 22/22, the full local gate and hosted Rust on Windows, Ubuntu and macOS. Frontend, both cross-checks, serving and guards also pass. Final bounded three-user file and natural `qwen3:8b` replay now passes on `532c3b7d`; mixed GUI/CLI, broader fault/privacy coverage and native Windows acceptance remain open. All visuals stay local; AWS product-transfer and native CUA approvals remain pending. [Evidence](evidence/shared-daemon-acceptance-20260922.md) preserves bounded results and excluded fixtures.

Development apps are packaged and ad-hoc signed, not launched; native CUA approval remains pending. [Current acceptance](evidence/shared-daemon-532c3b7d-20260923.md) and [Linux report](evidence/linux-532c3b7d-20260923.md) preserve exact scopes.

The [refreshed Linux pair](evidence/linux-dd70051e-20260923.md) builds from
`dd70051e` and passes fresh ordinary-UID lifecycle checks. The [prepared QA
apps](evidence/prepared-gui-7ab40c81-20260923.md) now embed the exact `7ab40c81`
native pair and pass signature checks; their runtime acceptance remains pending.
