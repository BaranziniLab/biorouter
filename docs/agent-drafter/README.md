# Agent Drafter

Agent Drafter is BioRouter's app-authoring MCP extension: it builds **BioRouter apps**, each a served TypeScript front-end wired to its own per-app BioRouter agent over a WebSocket. This folder holds the subsystem's design map — what an app is, how the Apps SDK v2 surface is organised, and how protocol, capabilities, themes, export modes and multi-agent orchestration fit together — plus the frozen 100-spec corpus and runbook used to stress-test it.

Come here when you want to understand the Agent Drafter subsystem as a whole, or when you are about to run a test campaign against it. Two neighbours own adjacent ground. [`docs/apps-sdk/`](../apps-sdk/README.md) holds the SDK contract itself — every `br.*` signature, the manifest schema, the frame tables, the v2 design and its phase roadmap; go there when you need to know what actually ships or how to author an app. [`docs/history/agent-drafter-testdrive-100/`](../history/agent-drafter-testdrive-100/README.md) holds the archived evidence from the one large campaign that consumed this folder's corpus — per-app verdicts, audits, and the remediation that closed it; go there for what a run found, not for how to run one.

## Documents

| Document | What it covers |
|---|---|
| [BioRouter Apps platform design](apps-platform-design.md) | The design overview of the Agent Drafter subsystem — the nine SDK v2 pillars, protocol v2, the capability matrix, archetypes, theme packs, export modes, multi-agent orchestration, the `biorouter apps` CLI and the testing story. Its first half is current and matches the shipped code; its clearly marked second half is a deliberately retained historical record of the v1 redesign and the app-building campaigns that produced the current design. |
| [Built apps hardening: handoff](built-apps-hardening-handoff.md) | Exact checkout and PR routing for the October 2026 Launch, build-chat and Export fixes, review dispositions, acceptance evidence and design limits. |
| [Built apps hardening: verification](built-apps-hardening-verification.md) | Current integration ledger, isolated QA profile, workstream reports, and final acceptance evidence. |

## Subdirectories

- **[`testing/`](testing/README.md)** — the test-drive workload for Agent Drafter: the operational runbook and the frozen 100-spec corpus it consumes. It carries its own index; its two documents are summarised below.

| Document | What it covers |
|---|---|
| [Agent Drafter 100-app test-drive runbook](testing/app-test-drive-runbook.md) | The operational procedure for driving Agent Drafter across the 100 app specs — bring a daemon up, make the agent author each app, drive the result in a browser against a functional and an aesthetic rubric, and log every defect. Current, with one rotted section: the git worktree its "Where the code lives" section requires no longer exists, since the Apps SDK v2 primitives now live in the main tree; everything else runs from an ordinary checkout. |
| [Hundred-app test specs for Agent Drafter](testing/hundred-app-test-specs.md) | The frozen corpus of 100 ambitious app briefs — concept, theme, layout, agent profiles, declared actions, signals, bidirectional loop, platform integration — expressed in still-shipping SDK v2 primitives. Current and reusable, but locked: specs are cited by number in run results, so amend a brief in place rather than renumbering. |

## Related documentation

- [Apps SDK reference](../apps-sdk/sdk-reference.md) — the territory to this folder's map; the authority on what actually ships and what is only partially realised.
- [Apps SDK v2 design](../apps-sdk/v2-design.md) — the design of record for the nine pillars that the platform design summarises.
- [Apps SDK v2 phase roadmap](../apps-sdk/v2-phase-roadmap.md) — how those pillars were sequenced into six independently shippable phases.
- [100-app Agent Drafter test drive](../history/agent-drafter-testdrive-100/README.md) — the archived campaign that ran this folder's corpus, with per-app verdicts and the defects it exposed.
