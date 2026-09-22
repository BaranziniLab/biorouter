# BioRouter Crew research and implementation handoff

Start with the [comprehensive implementation plan](implementation-plan.md), revised September 22 after the user's decisions. Sections 1, 3, 6, 9 and 13 now define no-admin home-based deployment, 2–50-user labs, the cluster Public/Private toggle, shared MCP/SSH capabilities and real three-user dev-app acceptance testing. It incorporates the requested SSH/MFA/jump-host support, simple Linux/text storage preference, human and owned-agent collaboration, files, teams/channels and mandatory private/public boundaries.

| Artifact | Contents |
|---|---|
| [Implementation plan](implementation-plan.md) | Architecture, data/protocol/storage contracts, permissions, UI, deployment, phased implementation and acceptance gates |
| [Platform research](platform-research.md) | Original five options plus Matrix, Zulip, Mattermost, Rocket.Chat, Tinode and NATS/native; primary sources, licenses and limitations |
| [Current architecture/privacy](architecture-privacy.md) | Verified BioRouter source paths/lines; reusable extension/agent/session seams and security gaps |
| [Current SSH/UI architecture](ssh-ui-architecture.md) | Actual connection, navigation, file and stream support; new work and MFA design |
| [Feasibility report](feasibility-report.md) | Measured results on Narrows, Leo and one disposable AWS fixture; explicit untested boundaries |
| [Institutional probe evidence](institutional-host-smoke.json) | Machine-readable final results and 11 primitive checks per host |
| [AWS probe report](aws-identity-smoke-report.md) | Two real Unix accounts, simulated jump route, binary transfer, history replay and verified teardown |
| [Probe scripts](smoke/) | Small reproducible synthetic tests; not production Crew code |
| [Privacy review](review-privacy.md) and [SSH review](review-ssh.md) | Independent review findings and resolution records |

Baseline `314f3b268c24663a8696dbbbd5aa76a171d0fab8`; branch `codex/biorouter-crew`; worktree `/Users/wgu/.codex/worktrees/biorouter-crew/BioRouter`. Application source was not modified. The AWS fixture was terminated and cleaned up. Institutional probes made only temporary synthetic writes and removed them.

Current recommendation: build an ordinary-user-hosted, single-writer Crew broker with home-based JSONL history/files, native OpenSSH bridges and owner-scoped workers. No sudo, administrator provisioning or special service account is required; the hosting account is explicitly trusted with stored plaintext. The real-app AWS/three-user testing and built-in conversational SSH manager are planned release gates. The critical implementation gap is enforceable ownership and data isolation, not the ability to stream chat text over SSH. Real institutional MFA and production privacy controls remain acceptance work.

Historical feasibility validation (before the rootless revision): final institutional probe run returned 0 on both hosts; AWS run returned 0 and exact resource teardown was independently rechecked. Both independent design reviews have resolution records. Five Python files parsed, two evidence JSON files parsed, shell syntax and local Markdown links checked, `git diff --cached --check` passed, and `source bin/activate-hermit && cargo fmt --all -- --check` passed. No Rust/Electron application build or test suite was run for this research-only change.

The source/platform reports and their original review records remain research snapshots. Their earlier administrator-managed deployment recommendations are superseded by the updated implementation plan. Existing successful probes are not evidence that the new rootless installer, cluster toggle, MCP tools or three-user dev-app workflows are implemented.

September 22 revision validation: two independent reviewers checked the updated rootless design and cohesive dev-app testing contract; current-owner invitation/transfer semantics were corrected. Documentation links, code-fence balance and whitespace were checked. No new AWS fixture, application implementation or real-app test was executed for this documentation update.
