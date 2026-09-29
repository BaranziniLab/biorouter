# Crew build campaign

This folder records the campaign that designed, built and tested Crew, from the first research on 2026-09-21 to the merge into `main` on 2026-09-27. It happened and it shipped: PR #366 merged as `5f53ac451`, with hosted CI green on its head `ead0498d6` and no human review recorded on GitHub. It still matters as provenance: the plans say why Crew is shaped the way it is, the reviews say what was checked, and the evidence says which build each result holds for. None of it describes Crew today. For how Crew works now, read the [Crew design documents](../../crew/design/README.md) (the broker protocol, the naming design, the UI specification, the CLI guide, the SSH hop policy and the Linux build) and the [Crew user manual](../../crew/README.md).

## How the campaign ran

- **Research and plan, 2026-09-21 and 22.** Source investigations of BioRouter's existing architecture and SSH support, a survey of chat platforms, probes of two institutional hosts and one disposable AWS fixture, and two independent reviews led to the [implementation plan](implementation-plan.md): a small rootless broker reached over each member's own SSH login, owned agents, and mandatory private and public boundaries. Its §15 made daemon-owned parity between `biorouter crew` and the desktop a completion requirement.
- **Build and first acceptance, 2026-09-22 and 23.** The broker, the daemon's Crew core and routes, the native CLI and a first desktop view were built and checked by the bounded runs under [evidence](evidence/README.md) and the reports below, recorded in the [status ledger](implementation-status.md) and the [validation report](validation-report.md).
- **Resumed scope, 2026-09-23 to 25.** [Plan §16](implementation-plan.md#16-resumed-scope-gui-redesign-and-human-readable-identity-2026-09-23) added a Slack-like desktop redesign and people, teams and channels named instead of numbered, with joining by invitation and device code. Every §16 package was implemented, and acceptance ran as four live QA rounds with fresh novice critics (unaided task rate 39%, then 57%, 85% and 91%) and seven evidence-grade lanes on the final build `461f7899`. The [redesign acceptance evidence](evidence/ui-redesign-acceptance-2026-09.md) records what passed, the one failed assertion and what remains unverified.
- **Merge, 2026-09-27.** The [handoff's merge section](handoff-2026-09-24.md#at-the-merge-2026-09-27) records what became of each pre-merge item, including the security-sensitive commits that asked for human review.
- **After the merge, 2026-09-27 to 29.** A live QA of the merged build on a disposable AWS fleet, with a static audit beside it, was followed by two rounds of fixes on branch `claude/crew-qa-fixes-2026-09-27`. The second round (wave 2) changed behaviour the design documents pinned, and each changed row there says *Amended by wave 2* and names the QA finding. The design documents then moved out of `docs/research/`, which holds studies of systems outside this repository, into `docs/crew/design/`, and these records moved here, on 2026-09-28. A second live run on a new fleet found 90 more problems, a third round of fixes followed, and a final live round passed 80 of its 85 items with no P0 or P1. The fixes are in PR #377. The [merge QA evidence](evidence/merge-qa-2026-09-27.md) records all of it, finding by finding.

**Paths inside these records.** Commands and file paths that named the folder's old home under `docs/research/` were rewritten to this one when the folder moved, so the probe scripts can still be run as written; nothing else in a record changed, and paths outside this repository are as they were recorded.

## Plans, ledgers and handoffs

| Record | What it holds |
|---|---|
| [Implementation plan](implementation-plan.md) | Architecture, data, protocol and storage contracts, permissions, UI, deployment, phased implementation and acceptance gates, then §16's resumed scope, its work packages and the live QA rounds |
| [Acceptance status](implementation-status.md) | The feature by service and interface parity ledger, the G01 to G15 release gates, the naming slices and decisions log, and artifact provenance, up to the merge |
| [Resume handoff](handoff-2026-09-24.md) | What the merge settled, the close-out handoff (what was done, final artifacts, gates, deferred items, the human-review list), then the campaign record (work packages, workstreams, decisions log) above the 2026-09-24 pause receipts it supersedes in part |
| [Native CLI source plan](cli-parity-source-plan.md) | The source inventory and proposed command, client and lifecycle seams for the native CLI, and the map of the shared services that implemented them |
| [Desktop integration handoff](desktop-integration-handoff.md) | The build and launch contract for the three-client desktop runs; not evidence that a launch passed |

## Research and reviews

| Record | What it holds |
|---|---|
| [Existing architecture and security boundaries](architecture-privacy.md) | The 2026-09-21 source investigation: BioRouter's reusable extension, agent and session seams, the security gaps, and the implementation sequence it proposed |
| [SSH, identity, desktop and file investigation](ssh-ui-architecture.md) | The 2026-09-21 source investigation of connection, navigation, file and stream support, and the new work and MFA design Crew needed |
| [Platform research](platform-research.md) | The original five options plus Matrix, Zulip, Mattermost, Rocket.Chat, Tinode and NATS against a native broker; primary sources, licenses and limitations |
| [Feasibility report](feasibility-report.md) | Measured probe results on the Narrows and Leo hosts and one disposable AWS fixture, and the boundaries left untested |
| [Privacy review](review-privacy.md) and [SSH review](review-ssh.md) | Independent reviews of the first plan: findings and how each was resolved |
| [Broker source review](independent-review-broker.md) | The 2026-09-22 independent review of the broker and its lifecycle: findings and the corrected source for each |
| [Transport, server and UI source review](independent-review-transport-server-ui.md) | The 2026-09-22 independent review of the SSH transport, daemon routes and desktop UI: findings and recheck state |

## Evidence

| Record | What it holds |
|---|---|
| [Evidence records](evidence/README.md) | Curated summaries of bounded live runs, each with its source revision and scope, and the redesign's acceptance evidence |
| [Validation report](validation-report.md) | Exact build and check commands, selected test counts and artifact hashes, one appended section per run |
| [Three-client app evidence](crew-ui-acceptance-report.md) | Isolated Electron clients on the pre-redesign desktop: human collaboration, files, PAM sign-in and agent workflow observations |
| [Provider boundary evidence](local-provider-boundary-report.md) | A public positive control and private synthetic canary refusals on the real provider path |
| [Adversarial validation](adversarial-validation.md) | Ownership, revoked uploads, writer locking and journal faults |
| [50-user soak](local-real-uid-50-soak-report.md) | Real Unix identities, load metrics and restart qualifications |
| [Invariant coverage](regression-coverage-map.md) | I01 to I24 evidence plus P01 to P10 parity regressions and the redesign's suites, with the interface and acceptance gaps, at the close-out |
| [AWS probe report](aws-identity-smoke-report.md) | Two real Unix accounts, a simulated jump route, binary transfer, history replay and verified teardown |
| [Institutional SSH compatibility](institutional-ssh-compatibility.md) | Read-only metadata and kernel capability probes of the two institutional SSH targets, 2026-09-22 |
| [Linux SSH PAM probe](linux-ssh-pam-test-report.md) | A synthetic keyboard-interactive PAM probe against a disposable container; not production MFA |
| [Local Linux SSH fixture](local-linux-ssh-fixture-report.md) | The localhost Docker fixture with three synthetic Unix users that the earlier QA used |
| [Local multi-hop SSH probe](local-multihop-ssh-test-report.md) | Four localhost `sshd` listeners with distinct host keys: `ProxyJump`, cancellation and changed-key refusals |
| [Local Linux CLI and helper canary](local-linux-cli-canary-report.md) | Linux ARM64 broker, journal and peer-credential canaries on a local container |
| [QA fixture integrity](qa-fixture-integrity.md) | Why the first Bob enrollment attempt is excluded: the fixture's saved connection was altered by hand |

Three files here are raw data the reports cite, not prose: [`institutional-host-smoke.json`](institutional-host-smoke.json) holds the institutional probes' final results and 11 primitive checks per host, and [`crew-smoke-20260922T030933Z-8c3c8b.json`](crew-smoke-20260922T030933Z-8c3c8b.json) and [`crew-three-user-20260922T094718Z-fbf9dd51.json`](crew-three-user-20260922T094718Z-fbf9dd51.json) record the two AWS fixture runs. [`smoke/`](smoke/) holds the probe scripts that produced them and the later local runs: small reproducible synthetic tests, not production Crew code, with their own notes in [`smoke/aws/`](smoke/aws/README.md) and [`smoke/fixtures/`](smoke/fixtures/README.md).

## Checkpoints before the redesign

These were the ledger's headline results before work resumed on 2026-09-23, kept as they were written.

Earlier `5455ebf9` passes the full local gate, native and Linux builds and bounded shared-CLI deterministic and natural `qwen3:8b` Crew-tool acceptance. Merged checkpoint `3dac3695` is followed by API and docs `8be945c2`; the merged UI passes 562 files, 6,366 tests and 19 skips, typecheck and 60 affected tests. See [the status ledger](implementation-status.md).

Runtime-qualified source `532c3b7d` commits reviewed native continuation recovery and observer backlog corrections. Valid fresh-process CLI 11, daemon-client 16 and observer 12 regressions pass, and the full gate and native non-test build pass. Linux build and lifecycle, actual PTY leave, abandon and takeover, and bounded 60-ID backlog, fairness and slot replay pass. Published baseline `7ab40c81` has a passing native build and bounded SSH and Unicode smoke; its daemon is byte-identical to `532c3b7d`. Test-only portability corrections in `dd70051e` pass integration 42/42, core 22/22, the full local gate and hosted Rust on Windows, Ubuntu and macOS. Frontend, both cross-checks, serving and guards also pass. The final bounded three-user file and natural `qwen3:8b` replay passed on `532c3b7d`; mixed GUI and CLI, broader fault and privacy coverage and native Windows acceptance stayed open. All visuals stayed local; AWS product-transfer and native CUA approvals were pending. [Evidence](evidence/shared-daemon-acceptance-20260922.md) preserves bounded results and excluded fixtures.

Development apps were packaged and ad-hoc signed, not launched; native CUA approval was pending. [The `532c3b7d` acceptance](evidence/shared-daemon-532c3b7d-20260923.md) and [Linux report](evidence/linux-532c3b7d-20260923.md) preserve exact scopes. The [refreshed Linux pair](evidence/linux-dd70051e-20260923.md) builds from `dd70051e` and passes fresh ordinary-UID lifecycle checks, and the [prepared QA apps](evidence/prepared-gui-7ab40c81-20260923.md) embed the exact `7ab40c81` native pair and pass signature checks; their runtime acceptance was pending.

## Related documentation

- [Crew design and reference](../../crew/design/README.md): how Crew's broker, names, interface, CLI, SSH rules and Linux build work now
- [Crew user manual](../../crew/README.md): how to use Crew, for lab members, hosts and IT staff
- [Historical records](../README.md): the other campaigns in this archive
- [Privacy tiers](../../security/privacy-tiers.md): the privacy rules Crew's boundaries rest on
