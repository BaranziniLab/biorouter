# Agentic-loop improvement proposal lenses

This folder holds the three source brainstorms of the BioRouter agentic-loop improvement
review of 2026-07-12. Each file reads the same evidence base — the ten subsystem reviews and
the four comparison chapters in the parent folder — through a single concern: performance,
robustness, or usability. Between them they contain 132 proposals (48 performance, 50
robustness, 34 UX).

**This work happened, and it is finished.** All three lenses were merged and deduplicated
into the master register as `BR-1` … `BR-67`, and that merged programme was then implemented
and landed on `main`. Flagship items from these lenses shipped: shadow-git checkpoints and
rewind (`BR-43`), the `SharedMcpPool` (`BR-54`), background compaction (`BR-12`), the
single-turn server lock (`BR-33`), the repo map (`BR-1`) and read-only auto-approve
(`BR-18`). These files are kept for the record — the reasoning behind a `BR-NN` change, and
the ideas that were considered and dropped — not as an open work queue. **The current truth
about how the loop behaves lives in [the agent loop](../../../agent-loop/README.md)**, and
the authoritative list of what was actually adopted is
[the improvement proposals register](../improvement-proposals.md).

Come here only when you are tracing *why* a particular change was proposed, or looking for an
idea that was raised but never merged into the register. If you want to know what shipped, go
to the register. If you want the diagnosis these proposals responded to, go to
[the subsystem reviews](../subsystem-reviews/core-loop-and-tool-dispatch.md). If you want to
know how the work was sequenced and gated, go to
[the fix campaign](../../agent-loop-campaign/README.md).

> **Note.** `P-NN` numbers are **local to each file** — every lens restarts at `P-1`, so
> `P-3` in the performance lens is a different proposal from `P-3` in the robustness lens.
> The master register disambiguates them as `performance P-3`, `robustness P-3` and so on.
> `BR-NN` numbers are the merged register ids.

## Documents in this folder

| Document | What it covers |
|---|---|
| [Improvement proposals — Performance and efficiency](performance.md) | 48 proposals on latency, token efficiency, prompt caching, parallel tool execution, compaction cost, cheaper-model delegation, streaming, startup time and redundant-context elimination. |
| [Improvement proposals — Robustness and safety](robustness.md) | 50 proposals on loop and stuck detection, error-streak handling, checkpoints and undo of agent edits, crash/restart survival, permission and guardrail gaps, hook event coverage, sandboxing, and conversation-invariant edge cases. |
| [Improvement proposals — Usability, UX and agent ergonomics](ux.md) | 34 proposals covering what the user sees and controls (plan modes, progress and todo visibility, approval fatigue, resume and undo affordances) and agent ergonomics (tool descriptions, repo maps, verification feedback loops, done-ness signals, self-repair). |

Every proposal in all three files carries the same seven fields — Problem, Proposal, Inspired
by, Affected code, Impact, Effort, Risk — and cites the review that establishes the gap.
Effort is graded S (hours) / M (days) / L (weeks). Each file opens with an evidence-base table
mapping the short paths it cites (`internal/core-loop.md`, `compare/context.md`) to the
documents' current locations, and a cross-lens table naming the proposals that another lens
also raised.

## Related documentation

- [Improvement proposals register](../improvement-proposals.md) — the merged, deduplicated
  `BR-1` … `BR-67` programme these three lenses became; the authoritative record of what was
  adopted.
- [BioRouter agentic loop review](../README.md) — the executive report that frames all ten
  subsystem reviews, the eight external tool reports and these three lenses.
- [Agent-loop fix campaign](../../agent-loop-campaign/README.md) — how the merged proposals
  were sequenced into waves, gated and landed on `main`.
- [The agent loop](../../../agent-loop/README.md) — current documentation for the loop as it
  behaves today, including the designs that came out of this campaign.
