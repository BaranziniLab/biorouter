# Application Drafter testing

This folder holds the **inputs and procedure** for stress-testing Application Drafter, BioRouter's app-authoring MCP extension. It contains two documents that work as a pair: a frozen corpus of 100 ambitious app briefs, and the operational runbook that consumes them — bring a daemon up, make the agent author each app, drive the result in a browser against a functional and an aesthetic rubric, and log every defect.

Come here when you are about to **run** an Application Drafter test campaign and need the workload and the method. Go elsewhere if you want something adjacent: how the subsystem is designed lives in [the Apps platform design](../apps-platform-design.md) one level up; the `br.*` and `ui_*` API surface the briefs are written in lives in [`docs/apps-sdk/`](../../apps-sdk/sdk-reference.md); and the *results* of campaigns already executed — per-app verdicts, audits, remediation — are archived under `docs/history/`, not here. Nothing in this folder is a record of work done; both files are reusable material for the next run.

## Documents in this folder

| Document | What it covers |
|---|---|
| [Hundred-app test specs for Application Drafter](hundred-app-test-specs.md) | The frozen corpus of 100 app briefs — concept, theme, layout, agent profiles, declared actions, signals, bidirectional loop, platform integration — used as the stress-test workload. Current and reusable: every brief is expressed in still-shipping SDK v2 primitives. The corpus is **locked** — amend a brief in place rather than renumbering, inserting or deleting entries, because specs are cited by number in run results. |
| [Application Drafter 100-app test-drive runbook](app-test-drive-runbook.md) | The operational runbook for driving Application Drafter across that corpus: environment setup, the per-app authoring loop, the rubrics, and the findings log. Current, with one rotted section: it was written 2026-07-12 against a `feat/apps-sdk-v2` git worktree that no longer exists, so the worktree requirement in its "Where the code lives" section is void; the SDK v2 primitives now sit in the main tree and everything else runs from an ordinary checkout. |

Read the runbook before working through any brief in the corpus. It encodes hard-won operational detail — ports, keys, gotchas — from a real run.

## Identifier scheme

Specs are numbered `spec-NNN` (spec 1 is `spec-001`), and apps authored from them are named `spec-NNN-<slug>`, for example `spec-001-variant-tribunal`. The runbook numbers its own sections `§0`–`§10` and gives each rubric check the section number as an ID (`5.2` = "It is not a chatbot"). Both schemes are cited verbatim by the per-app result files that executed runs produce, which is why neither may be renumbered.

## Related documentation

- [Applications platform design](../apps-platform-design.md) — the design of the subsystem these tests exercise; read it as background before a campaign.
- [BioRouter Apps SDK v2 reference](../../apps-sdk/sdk-reference.md) — the manifest schema, `br.*` runtime, `ui_*` tools and frame protocol that the 100 briefs are written against.
- [100-app Application Drafter test drive](../../history/agent-drafter-testdrive-100/README.md) — the archived evidence set for the campaign this runbook drove: per-app rubrics, three cross-cutting audits, and the six-wave remediation that closed it.
- [Application Drafter stress test — 100 sophisticated agentic apps](../../history/agent-drafter-stress-test/README.md) — an earlier completed campaign, and the trail for why each `H<n>` drafter fix exists.
