# Competitive comparison chapters

This folder holds the four comparison chapters of the 2026-07 agentic-loop review: BioRouter's agent loop measured against nine other open-source coding agents — Goose, Cline, OpenCode, Pi, Aider, OpenHands, Codex CLI, Gemini CLI and Claude Code. All four chapters were written on 2026-07-12 and are kept for the record, not as current guidance.

The review did happen, and its findings became the `BR-1`…`BR-67` fix campaign, which has since been implemented and merged — so every chapter here is marked **Superseded**: the BioRouter column no longer describes the system. The nine competitor columns remain a 2026-07 snapshot and have not been re-verified since. For what actually shipped, and therefore the current truth, read [the agent-loop campaign outcome report](../../agent-loop-campaign/outcome-report.md) and the per-wave reports it links.

Come here when you want to know how BioRouter's loop was positioned against the field at that date, or why a particular `BR-NN` proposal was raised — each chapter closes with the recommendations that fed the register. You are in the wrong folder if you want the inward-looking analysis of BioRouter's own code, which lives in [`../subsystem-reviews/`](../subsystem-reviews/); the per-tool research reports on the nine competitors, which live in [`../../../research/coding-agent-landscape/`](../../../research/coding-agent-landscape/README.md); or the record of the fixes themselves, which lives in [`../../agent-loop-campaign/`](../../agent-loop-campaign/README.md). The chapters cite all three.

Two conventions run through every file. `BR-NN` identifiers are proposal numbers from this review, defined in full in [the improvement proposals register](../improvement-proposals.md). Gap citations of the form *(compaction review, gap #1 — `mod.rs:290-305`)* name the subsystem review that established a finding, its numbered gap, and the source lines that review cited; those line positions have since drifted.

## Chapters

| Document | What it covers |
|---|---|
| [Context injection, system prompts and environment awareness](context-and-prompts.md) | Chapter 1 — how the ten agents build their system prompts, read project-context files, inject context over time, and give the model awareness of its surroundings. Its central claim, "no repo map — BioRouter's single largest gap", was fixed by BR-1. |
| [Compaction, memory and session continuity](compaction-and-memory.md) | Chapter 2 — compaction triggers and strategy, what survives summarization, token counting, cross-session memory, and session persistence/resume. Its headline finding, "no recent-turn verbatim window", was fixed by BR-10. |
| [Tool loop, long-running tasks, checkpoints and verification](execution-and-verification.md) | Chapter 3 — tool dispatch and result flow, background processes, subagents, checkpoints/undo/git integration, and self-verification. Its two loudest claims, "no checkpoints/undo" and "no automatic post-edit verification", were fixed by BR-43 and BR-47. |
| [Hooks, permissions, guardrails and loop detection](safety-and-guardrails.md) | Chapter 4 — the safety surface: lifecycle hooks, the permission/approval flow, LLM judges, sandboxing, dangerous-command detection, and repetition/stuck detection. The "computed-but-discarded / dead-code" thesis it is built on has been resolved, so most of its "behind" rows are now wrong. |

## Related documentation

- [BioRouter agentic loop review](../README.md) — the executive report these chapters belong to, with the 14 review questions and the index of all 28 sub-reports.
- [The improvement proposals register](../improvement-proposals.md) — `BR-1`…`BR-67`, the consolidated list every chapter's recommendations fed into.
- [The agent-loop campaign](../../agent-loop-campaign/README.md) — the campaign that implemented those proposals; read its outcome report for the state that replaced the BioRouter column here.
- [Coding-agent landscape research](../../../research/coding-agent-landscape/README.md) — the per-tool reports grounding every external claim in these chapters.
