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

Current product HEAD is `9a3957d6`, after reviewed transfer/GUI/API checkpoint `67a32a07` with focused server and GUI-handler tests passing. Transport retirement is committed in current HEAD `9a3957d6`; nine regressions, strict core Clippy and formatting pass. Runtime rebuild, full gates and live changed-target confirmation remain pending. Pinned native/Linux artifacts, the full gate and 6,256-test desktop pass belong to earlier `906bf68b`; its bounded live results do not validate newer source. See [status](implementation-status.md), [validation](validation-report.md) and [CLI evidence](evidence/crew-cli-observer-context-20260922.md).

The daemon owns connection, authentication, transfer, task and observer services; CLI/GUI adapters preserve the Proven-only human gate. GUI preparation builds/typecheck pass without launch; actual interaction awaits native approval. Windows source is integrated and cross-compiles, but native qualification is open. [PR #366](https://github.com/BaranziniLab/biorouter/pull/366) still has hosted head `3145dfc5` with 22 successes and one skip, excluding newer local commits. Screenshot-publication, AWS source-export and native CUA approvals remain pending. No blocked action was retried through a workaround. See [accepted contracts](implementation-plan.md#accepted-bootstrap-and-credential-design) and the [remaining gates](implementation-status.md#release-gates).

The implementation uses an ordinary-user-hosted, single-writer Crew broker with home-based JSONL history/files, native OpenSSH bridges and owner-scoped workers. No sudo, administrator provisioning or special service account is required; the hosting account is explicitly trusted with stored plaintext. Three real local Linux users and three isolated desktop clients exercise the implementation. AWS product validation is blocked on the separately requested source-export approval; the earlier AWS feasibility probe is not a substitute. Real institutional MFA and independent Windows/Linux client acceptance remain separate gates. Narrows NFS/kernel limits and Leo Landlock ABI 1 prevent current supported deployment/execution; trusted hosting and SSH alone do not establish HIPAA compliance.

Historical feasibility validation (before the rootless revision): final institutional probe run returned 0 on both hosts; AWS run returned 0 and exact resource teardown was independently rechecked. Both independent design reviews have resolution records. Five Python files parsed, two evidence JSON files parsed, shell syntax and local Markdown links checked, `git diff --cached --check` passed, and `source bin/activate-hermit && cargo fmt --all -- --check` passed. No Rust/Electron application build or test suite was run for this research-only change.

The source/platform reports and their original review records remain research snapshots. Their earlier administrator-managed deployment recommendations are superseded by the updated implementation plan. Those historical probes do not establish current implementation acceptance; use the newer reports and their exact artifact hashes.

September 22 revision validation: two independent reviewers checked the updated rootless design and cohesive dev-app testing contract; current-owner invitation/transfer semantics were corrected. Documentation links, code-fence balance and whitespace were checked. No new AWS fixture, application implementation or real-app test was executed for this documentation update.
