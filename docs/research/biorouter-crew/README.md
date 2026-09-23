# BioRouter Crew implementation and evidence

Start with the [comprehensive implementation plan](implementation-plan.md), revised September 22 after the user's decisions. **Section 15 makes daemon-owned native `biorouter crew`/GUI functional parity a completion requirement**, including terminal MFA/human approvals, shared transfers/tasks, CLI-only acceptance with Crew Electron clients fully closed, then mixed GUI/CLI collaboration among three real Unix users. Earlier GUI-first progress does not measure this scope. Sections 1, 3, 6, 9 and 13 define no-admin home-based deployment, 2–50-user labs, the cluster Public/Private toggle, shared MCP/SSH capabilities and real three-user dev-app acceptance testing. It incorporates the requested SSH/MFA/jump-host support, simple Linux/text storage preference, human and owned-agent collaboration, files, teams/channels and mandatory private/public boundaries.

| Artifact | Contents |
|---|---|
| [Implementation plan](implementation-plan.md) | Architecture, data/protocol/storage contracts, permissions, UI, deployment, phased implementation and acceptance gates |
| [Acceptance status](implementation-status.md) | Feature-by-service/interface parity ledger, G01–G15 release gates, remaining blockers and artifact provenance |
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

Published `5455ebf9` passes the full local gate, native/Linux builds and bounded shared-CLI deterministic and natural `qwen3:8b` Crew-tool acceptance. Merged checkpoint `3dac3695` is followed by API/docs `8be945c2`; the merged UI passes 562 files/6,366 tests/19 skips, typecheck and 60 affected tests. See [current status](implementation-status.md).

Uncommitted native continuation recovery and observer backlog corrections are source-reviewed. Observer regressions pass 12 tests, but final gates, new paired artifacts and live backlog replay remain pending. Full CLI/mixed-GUI, broader faults/privacy and native Windows remain open. All visuals stay local; AWS product-transfer and native CUA approvals remain pending. [Evidence](evidence/shared-daemon-acceptance-20260922.md) preserves bounded results and excluded fixtures.
