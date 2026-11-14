# Local dictation build campaign

> **What this is.** The record of how local dictation and the personal dictionary were planned and built, from the owner's first decisions on 2026-11-05 to the merge into `main` on 2026-11-14.
> **Status:** Historical record. Shipped: pull request #485 merged on 2026-11-14 through the batch pull request #501 (merge commit `4f86f4f1b`), with every check green on the batch head `071f5412a`.
> **Audience:** maintainers tracing a dictation decision (a DD, O, P or M number) or a pull request of the plan back to why it was made.

This happened and it shipped. Local speech to text runs on the person's own hardware on every Biorouter surface, with a personal dictionary that shapes recognition and follows the privacy tiers. The plan below is the build record: the owner's decisions, the design decisions with their reasons, and each pull request of the plan (PR-01 to PR-33, PR-07 and PR-32 withdrawn) with what it changed and the commit it landed in. It still matters when a change to dictation touches one of those decisions. How dictation works today is in the living documentation, [docs/dictation](../../dictation/README.md), and the manual pass per surface is still open in the [dictation test checklist](../../releases/dictation-test-checklist.md).

## How the campaign ended

- **Build, 2026-11-05 to 11-10.** The code pull requests landed on branch `feat/local-dictation` one at a time, each with its own hosted CI; the checklist's "CI on the branch" table records those runs.
- **Owner decisions before the merge, 2026-11-13.** The functions over clippy's length limit were split, the Dictionary's tools reached Claude Code and Codex chats, attaching the Dictionary from `POST /agent/add_extension` and `POST /agent/start` asks for the person, and Apple Lossless decodes (O-41). The plan records each one.
- **Merge, 2026-11-14.** The remaining pull requests landed together as one batch, which also found and fixed a decoder defect: a damaged `.m4a` made the MP4 reader reserve tens of gigabytes, which aborts the process on Linux (`4e5b6e96c`, `071f5412a`). The checklist's "CI at the merge" table records the batch's runs.

| Record | What it holds |
|---|---|
| [Implementation plan](implementation-plan.md) | The owner decisions (O), design decisions (DD), privacy rules (P), measurements (M) and risks, then every pull request of the plan with its files, its tests and the commit it landed in |

## Related documentation

- [Local dictation](../../dictation/README.md): how dictation, the speech models and the dictionary work now
- [Dictation test checklist](../../releases/dictation-test-checklist.md): the manual pass per surface and the CI record
- [Speech model benchmarks](../../research/voice-dictation/speech-model-benchmarks.md): the measurements behind the model choices
- [Historical records](../README.md)
