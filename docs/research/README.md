# Research

This folder holds **external research** — studies of software outside BioRouter, written to
inform BioRouter's own design. Its single topic today is the coding-agent landscape: nine
reports reviewing how other agentic coding tools build their feedback loops, each covering
the same ten dimensions so they can be read side by side.

Come here when you want to know **how somebody else solved a loop problem** — how Gemini CLI
detects a stuck agent, how Cline checkpoints edits, how Codex CLI sandboxes a command — before
designing the BioRouter equivalent. Go elsewhere if you want BioRouter's own behaviour: how
the agent loop works *today* lives in [`docs/agent-loop/`](../agent-loop/README.md), and the
point-in-time diagnosis of BioRouter's loop, the head-to-head comparison chapters and the
`BR-NN` proposals derived from that diagnosis live under
[`docs/history/agent-loop-review/`](../history/agent-loop-review/README.md). The reports here
describe *other projects* and cite no BioRouter source, which is why they stay current while
BioRouter changes around them.

## The `BR-NN` identifiers

Every report cites proposal numbers of the form `BR-1`, `BR-43`, `BR-67`. These are the
improvement proposals assigned by BioRouter's agentic-loop review; the full index is
[the improvement proposals register](../history/agent-loop-review/improvement-proposals.md).
A report naming a `BR-NN` number is the cited external source behind that proposal.

## Coding agent landscape

The [`coding-agent-landscape/`](coding-agent-landscape/README.md) subdirectory holds all nine
reports — [Aider](coding-agent-landscape/aider.md),
[Claude Code](coding-agent-landscape/claude-code.md), [Cline](coding-agent-landscape/cline.md),
[OpenAI Codex CLI](coding-agent-landscape/codex-cli.md),
[Gemini CLI](coding-agent-landscape/gemini-cli.md), [Goose](coding-agent-landscape/goose.md),
[OpenCode](coding-agent-landscape/opencode.md),
[OpenHands](coding-agent-landscape/openhands.md) and [Pi](coding-agent-landscape/pi.md). It
carries its own index describing each report and the `BR-NN` proposals each one became the
cited source for, so read that index rather than a second copy of it kept here.

Each report covers the same ten dimensions: system prompt and context injection, tool loop
mechanics, compaction and memory, hooks and extensibility, guardrails and permissions, loop and
stuck detection, long-running tasks and background processes, state tracking and checkpoints,
self-verification, and ideas worth stealing. All nine are marked **Current**: they describe
external projects, so BioRouter's own changes do not invalidate them. Several are explicitly
dated snapshots of fast-moving repositories, noted per report in that index.

Three are worth knowing about before you go.
[Claude Code](coding-agent-landscape/claude-code.md) is treated as the reference design the
whole corpus benchmarks against, and is the only report on a closed-source product, so it dates
fastest. [Gemini CLI](coding-agent-landscape/gemini-cli.md) covers an independent TypeScript
architecture and is one of the corpus's deepest reports.
[Goose](coding-agent-landscape/goose.md) is the odd one out: it reviews the agent whose design
influenced BioRouter most, and it is the repository's only detailed record of what Goose
changed in 2025 and 2026.

## Related documentation

- [Agentic loop review](../history/agent-loop-review/README.md) — the executive report these nine external reviews fed into, including the four head-to-head comparison chapters that score the agents against each other.
- [Improvement proposals register](../history/agent-loop-review/improvement-proposals.md) — the `BR-1`…`BR-67` index; the destination for every proposal number cited in this folder.
- [Agent loop](../agent-loop/README.md) — how BioRouter's own loop, context engineering, hooks and subagents work today, as opposed to how other tools do it.
- [Agent-loop campaign](../history/agent-loop-campaign/README.md) — the implementation campaign that acted on the gaps these reports identified.
