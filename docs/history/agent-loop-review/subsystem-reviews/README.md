# Subsystem reviews

This folder holds the ten inward-looking subsystem reviews of the 2026-07 BioRouter agentic-loop review — the chapters that read BioRouter's own agent-loop code and recorded what was wrong with it. The review did happen: it ran in July 2026 — the one chapter that pins a revision read commit `24cdc3a2` on `main`, and a second records the reading date 2026-07-12 — and the combined gap lists became the `BR-1`…`BR-67` improvement register, which was then implemented and merged. Every file here is therefore a **historical record of superseded code**, kept for the record and not as current guidance. Each one's context header names the specific `BR-NN` proposals that closed its gaps, so a reader can trace a finding forward; for what actually shipped, and therefore the current truth, read [the agent-loop campaign outcome report](../../agent-loop-campaign/outcome-report.md) and the per-wave reports it links.

Come here when you want to know *why* a particular `BR-NN` proposal was raised, or want the pre-campaign architecture narrative for one subsystem — several of these files describe machinery that still exists and still works the way they say, even where the gap list is spent. You are in the wrong folder if you want the outward-looking comparison against eight other coding agents, which lives in [`../competitive-comparison/`](../competitive-comparison/README.md); the `BR-NN` register itself, which lives in [`../improvement-proposals.md`](../improvement-proposals.md); or the record of the fixes, which lives in [`../../agent-loop-campaign/`](../../agent-loop-campaign/README.md).

> **Note.** The reviews disagree about what revision they read. Only [guardrails and permissions](guardrails-and-permissions.md) pins a commit (`24cdc3a2` on `main`); [self-verification and done-ness](self-verification-and-doneness.md) pins only the date 2026-07-12; [state awareness and version control](state-awareness-and-version-control.md) records the unrelated branch `ui-hardening-a11y-tests`; the rest record no revision at all. Treat every `file.rs:line` citation as a pointer to the right function, not an exact location.

## Documents

| Document | What it covers |
|---|---|
| [Compaction and context management](compaction-and-context-management.md) | The summarize-everything compaction strategy — the 0.8 trigger, the visibility-flag mechanism that makes compaction non-destructive, token counting, tool-pair edge cases and persistence — and thirteen gaps. |
| [Context injection and system prompt construction](context-injection-and-system-prompt.md) | How the agent assembles the system prompt and the message array: the three injection cadences, the MOIM ambient-context block, hint files and `@import`, and MCP instruction rendering — plus a critique of `system.md` and `desktop_prompt.md`, and eleven gaps. |
| [Core agent loop and tool dispatch](core-loop-and-tool-dispatch.md) | The reasoning loop end to end — one reply turn, tool dispatch and result flow, the `Conversation` invariant, provider retries, oversized responses and streaming — and ten gaps. The longest of the ten reviews. |
| [Guardrails, security and the permission system](guardrails-and-permissions.md) | The tool-call gauntlet: the four-inspector chain, permission modes and the approval flow, the security scanner, PII guardrails and the OSV malware check, and ten gaps. |
| [Hooks system](hooks-system.md) | BioRouter's Claude-Code-compatible hook system — the 13 wired event variants, command versus prompt hooks, the outcome and decision model, configuration and matchers — and thirteen gaps. |
| [Long-running tasks, background processes and scheduling](long-running-tasks-and-scheduling.md) | The four mechanisms for work that outlives a single tool call — background shell jobs, subagents, the cron scheduler and MCP elicitation — plus process tracking, what survives a daemon restart, and eleven gaps. |
| [Loop detection, repetition and stuck states](loop-and-stuck-detection.md) | The five layered mechanisms that bound runaway agent behaviour — the `RepetitionInspector`, the 100-iteration turn cap, cancellation, provider retry and `/goal` stall detection — and ten gaps. |
| [Self-verification, output validation and done-ness](self-verification-and-doneness.md) | The three code-level verification checkpoints — `final_output` JSON-Schema validation, workflow success-checks with retry, and the `/goal` judge — the argument that nothing enforced done-ness in interactive chat, and eleven gaps. |
| [Server reply flow and session lifecycle](server-reply-flow-and-session-lifecycle.md) | A GUI message traced through the `biorouterd` daemon: the SSE `/reply` route, session creation and resume, cancellation, the action-required approval pause, auth and concurrency, and eleven gaps. |
| [State, awareness, todos, goals and version control](state-awareness-and-version-control.md) | How the agent knows where it is and what it is doing — working-directory propagation, the todo blob, `/goal` state, cross-session memory, mistake signals and the SQLite session schema — and ten gaps. |

## How sibling documents cite these files

The reviews were renamed during the documentation cleanup, but citations elsewhere in the corpus still use the old short names. Each file's context header records its former name so a citation such as `compaction.md gap #N`, `context-injection.md gap #N`, `core-loop.md gap #N`, `guardrails-permissions.md #N`, `hooks.md #N`, `long-running.md gap #N`, `loop-detection.md #N`, `verification.md gap #N`, `server-flow.md gap #N` or `state-awareness.md gap #N` can be resolved to the file above. In every case the number refers to an item under that file's "Gaps and weaknesses" heading, not to its answer sections.

## Related documentation

- [The agentic-loop review executive report](../README.md) — the parent report that these ten chapters feed, plus the index of all 27 sub-reports in the review.
- [The improvement-proposals register](../improvement-proposals.md) — defines every `BR-NN` identifier the headers above cite as the fix for a gap.
- [The agent-loop fix campaign](../../agent-loop-campaign/README.md) — how the resulting work was sequenced into waves, and the record of every gate and merge decision.
- [Competitive comparison chapters](../competitive-comparison/README.md) — the outward-looking half of the same review, measuring the subsystems described here against eight other open-source coding agents.
