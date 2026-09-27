# Coding agent landscape

This folder holds nine external reviews of other coding agents, each examining how that
tool structures its agentic feedback loop across the same ten dimensions — system prompt
and context injection, tool surface, permissions, verification, compaction, checkpoints,
loop detection, hooks, extensions and delegation. They were compiled in July 2026 as the
research input to BioRouter's agentic-loop review, and each names the `BR-NN` proposals it
became the cited source for. `BR-NN` identifiers are proposal numbers from that review;
the register lives in
[the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

Come here when you want to know **how some other agent solved a loop problem** — Gemini
CLI's layered loop detection, Cline's shadow-git checkpoints, Codex CLI's Starlark command
policy — before designing BioRouter's own version. Two neighbours cover the adjacent
questions, and one of them is probably what you want instead. For the *comparison* of those
tools against BioRouter, and the proposals that came out of it, go to
[the agentic loop review](../../history/agent-loop-review/README.md); note that its
comparison chapters are marked superseded, since the campaign they diagnosed has since
shipped. For **how BioRouter's own loop works today** — context engineering, subagents,
hooks, the policy engine — go to [the agent loop](../../agent-loop/README.md). Nothing in
this folder describes BioRouter's behaviour.

Every report here is marked **Current**: they describe external projects, so BioRouter's
own changes do not invalidate them. They do age against their subjects, though. All nine
cite default branches rather than pinned commits, and most record only month-granularity
research dates, so re-verify line-level details against current upstream source before
acting on them.

## Documents

| Document | What it covers |
|---|---|
| [Aider](aider.md) | How Aider (`Aider-AI/aider`), the open-source terminal "AI pair programming" agent, structures its loop — repo map, git integration, lint/test-after-edit, the reflection loop, and the architect/editor split. Deliberately not a free-running autonomous agent; the cited source for the ranked repo-map design behind BR-1 and the post-edit reflection loop behind BR-47. |
| [Claude Code](claude-code.md) | Anthropic's terminal-native coding agent (CLI + IDE extensions + web/cloud), treated as the reference design this corpus benchmarks BioRouter against. The cited source for hook-model parity, tool-output offloading (BR-6) and lower-trust project files (BR-9). Pinned to the v2.1.x line and closed-source, so it will date faster than the open-source reports here. |
| [Cline](cline.md) | How Cline — an open-source autonomous agent shipping as a VS Code extension, a CLI and an SDK — implements its loop, emphasising shadow-git checkpoints, three-axis restore, the mistake tracker and the rules system. The source for the shadow-git checkpoint design that BR-43 implemented. Researched 2026-07-12 against a project that had just refactored into a monorepo, so expect drift. |
| [OpenAI Codex CLI](codex-cli.md) | OpenAI Codex CLI (`openai/codex`), a Rust agent workspace: per-model system-prompt files, the `execpolicy` Starlark command policy, OS sandboxing, and a self-maintained ranked memories layer. Architecturally the closest relative of BioRouter and Goose, which are also Rust agents, and the cited source for BR-3, BR-2, BR-19 and BR-21. |
| [Gemini CLI](gemini-cli.md) | Google's open-source terminal agent: the layered `LoopDetectionService`, a declarative TOML policy engine, shadow-git checkpointing with `/rewind`, and 30%-verbatim-tail compaction. A TypeScript architecture unlike BioRouter's Rust one, which makes it the cleanest reference in the corpus for a competitor built from scratch, and one of its deepest reports. Source for BR-10, BR-29/BR-30 and BR-43. |
| [Goose](goose.md) | Goose (Block, now stewarded by the Agentic AI Foundation), the agent whose design influenced BioRouter most. Emphasises what Goose added or changed in 2025 and 2026, with **Gap compared with Goose** callouts in the sections where BioRouter may lack a feature Goose added. The repository's only detailed record of those changes; check every gap item again before acting on it. |
| [OpenCode](opencode.md) | How OpenCode (SST, a TypeScript/Bun monorepo) implements its loop, emphasising its client/server split, private git-object-DB snapshots with `/undo` and `/redo`, and prune-before-summarize compaction. The cited source for the private git-object-DB checkpoint approach adopted in BR-43. Researched 2026-07-12. |
| [OpenHands](openhands.md) | How OpenHands (All Hands AI) implements its loop — the five-heuristic `StuckDetector`, the structured summarizing condenser that preserves task IDs, and risk-graded confirmation via a per-action `security_risk` field. Spans two repositories since the late-2025 SDK split. Source for the oscillation/action-error detection in BR-30 and the risk-graded permission model in BR-18. |
| [Pi](pi.md) | Pi's deliberately minimal loop (badlogic / Mario Zechner) — a ~1000-token system prompt, no MCP (Model Context Protocol), no subagents, session-tree branching, and typed extension hooks that can rewrite the prompt and the message array. A subtractive thesis: what you leave out matters more than what you put in. Source for per-directory Project Trust (BR-9) and the "split turn" compaction fallback (BR-11). |

## Related documentation

- [The agentic loop review](../../history/agent-loop-review/README.md) — the executive report these nine reports fed into, including the four-chapter competitive comparison that weighs them against BioRouter.
- [The improvement proposals register](../../history/agent-loop-review/improvement-proposals.md) — defines every `BR-NN` identifier cited in this folder's status headers.
- [The agent-loop fix campaign](../../history/agent-loop-campaign/README.md) — how the resulting proposals were sequenced and merged, when you want to know what actually shipped from a given report's idea.
- [The agent loop](../../agent-loop/README.md) — BioRouter's own loop as it works today, which is what most readers who land here by mistake are looking for.
