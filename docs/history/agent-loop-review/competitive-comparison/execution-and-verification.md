# Tool loop, long-running tasks, checkpoints and verification

> **What this is.** Chapter 3 of the four-part competitive comparison in the
> 2026-07 agentic-loop review: how BioRouter's tool dispatch and result flow,
> background processes, subagents, checkpoints/undo/git integration, and
> self-verification compared against nine other open-source coding agents.
> **Status:** Superseded — the BioRouter column no longer describes the system. Its
> two loudest claims were fixed after the review: "no checkpoints/undo" by BR-43
> (`crates/biorouter/src/checkpoint/`) and "no automatic post-edit verification" by
> BR-47 (`crates/biorouter/src/agents/post_edit_diagnostics.rs`); oversized-output
> handling (BR-6) and bounded parallelism also landed. For what shipped, read
> [the agent-loop campaign outcome report](../../agent-loop-campaign/outcome-report.md)
> and [the wave-1 checkpoints report](../../agent-loop-campaign/wave-reports/wave-1-checkpoints.md).
> **Audience:** developers working on the agent loop, tool dispatch and verification.

This chapter was written on 2026-07-12 as part of the agentic-loop review. Its
subject is the execution half of the loop: how tool calls are dispatched and
their results returned, what happens to work that outlives a single turn, whether
the agent can undo what it did, and whether anything checks that an edit was
correct. Read it as a snapshot of the state of the art at that date and of
BioRouter's position in it — not as a description of current BioRouter behaviour.

Three conventions used throughout:

- **`BR-NN` identifiers** are proposal numbers from the same review.
  [The improvement proposals register](../improvement-proposals.md) defines
  BR-1…BR-67 with Problem / Proposal / Affected code / Impact / Effort / Risk.
- **Gap citations** take the form *(core-loop review, gap #8)*. They name the
  subsystem review that established the finding and its numbered gap. The four
  grounding reviews for this chapter are
  [the core-loop and tool-dispatch review](../subsystem-reviews/core-loop-and-tool-dispatch.md)
  ("core-loop review"),
  [the long-running tasks and scheduling review](../subsystem-reviews/long-running-tasks-and-scheduling.md)
  ("long-running review"),
  [the server reply-flow and session-lifecycle review](../subsystem-reviews/server-reply-flow-and-session-lifecycle.md)
  ("server-flow review"), and
  [the self-verification and done-ness review](../subsystem-reviews/self-verification-and-doneness.md)
  ("verification review").
- **External claims** are grounded in that tool's report under
  [the coding-agent landscape research](../../../research/coding-agent-landscape/).

The reviewed BioRouter commit was not recorded in the original document.

## Comparison across ten agents

The table has one column per agent, in this order: BioRouter, Goose,
Cline, OpenCode, Pi, Aider, OpenHands, Codex CLI, Gemini CLI, Claude Code. It is
wide and scrolls horizontally.

> **Note.** The BioRouter column is superseded — see the status header. The nine
> competitor columns are a 2026-07 snapshot and have not been re-verified since.

| Aspect | BioRouter | Goose | Cline | OpenCode | Pi | Aider | OpenHands | Codex CLI | Gemini CLI | Claude Code |
|---|---|---|---|---|---|---|---|---|---|---|
| Tool dispatch model | Function-calling (MCP), name-routed | Function-calling (MCP) | Function-calling + apply-patch | Function-calling (typed parts) | Function-calling, 7 built-ins | **Plain-text edit formats**, parsed locally | Function-calling (event stream) | Function-calling (Responses API) | Function-calling (scheduler) | Function-calling, named built-ins |
| Parallel tool exec | Yes, unbounded `select_all` | Yes, `select_all` | Yes (prompt-driven batching) | Subagents run **sequential**; tools parallel | Preflight-then-concurrent | No (one edit batch/turn) | Yes, `ParallelToolExecutor` | Yes, **R/W-lock gated** | Yes, batched w/ ordering rules | Yes, batch + `PostToolBatch` |
| Result streaming | Yes, SSE deltas | Yes | Yes | Yes (SSE parts) | Yes (decoupled stream) | Yes (terminal) | Yes (`on_token`) | Yes (Responses SSE) | Yes | Yes |
| Oversized tool output | Temp-file dump >200k chars, no preview | Background tool-pair summarize (cutoff 10) | Output-limits handling | Prune tool bodies before summarize | Truncate to 2000 chars on compact | Fed inline | Truncated by condenser | `TruncationPolicy` truncation | **Truncate to disk + summary ref** | **Cap ~30KB → file + head preview** |
| Per-tool timeout | No; per-extension 300s only | — | 5-min approval timeout | maxLines/maxBytes caps | bash optional timeout | Sync blocking | — | background max 1h awaiter | `inactivityTimeout` on bg shell | Bash 2-min default/10-min ceiling |
| Background shell jobs | **Yes** (`background=true`, exit-truth) | Docs pattern | Yes (bg processes) | Experimental bg subagents | **No** (use tmux) | No (watch mode only) | Sandbox terminal | **Yes**, `awaiter` poller | Yes (`is_background`, PIDs) | **Yes**, `run_in_background`+Monitor |
| Process tracking / orphan cleanup | 3 disjoint registries, **in-mem, no reaping** | — | — | task_status only | — | — | — | thread-store | Background PIDs surfaced | `/tasks`, TaskStop |
| Subagents | Yes, blocking, sem-capped (8/64) | Yes, isolated, max_turns 25 | `spawn_agent`, read-only, non-recursive | `task`, sequential, 30-min TTL bg | **No** (spawn `pi` via bash) | No (architect/editor split) | `spawn`/`delegate`, parallel threads, max 5 | `spawn_agent` family + CSV fan-out | subagents, sep context, no recursion | `Agent` tool, **bg by default**, summary-only |
| Subagent result | Last text msg (lossy) or schema | Full/summary | text + token usage | child session | — | — | join() results | wait_agent | separate loop | 400-token summary |
| Checkpoints / undo | **None** | **None** (git-commit pattern) | **Shadow-git, 3 restore modes** | **Git-object-DB snapshots, /undo/redo** | Session-tree branching | git `/undo` per-commit | Event-store replay/fork | Git-aware, refuse destructive | **Shadow-git + /rewind** | **Auto checkpoints + rewind** |
| Git integration of edits | None documented | Manual commit-after pattern | Shadow repo, real .git untouched | Private object DB, no commits | Extension use-case | **Auto-commit per edit + attribution** | Workspace layer | `apply_patch` diffs, no auto-undo | shadow repo in home | Checkpoints + worktrees |
| Post-edit verification | **None automatic** (`analyze` manual) | None (recipe opt-in) | Prompt-driven | **LSP diagnostics + auto-format** | None | **Auto-lint + auto-test scoped to edits** | Critic/judge | Prompt: run tests after edit | Hook-composed | **LSP after-edit + PostToolUse hooks** |
| Enforced test-gate | Workflow-only success-checks | Recipe `SuccessCheck` | YOLO test-until-green | Model-judged | None | **Reflection loop (lint/test/malformed)** | Critic + goal-judge | Goal completion audit | `AfterAgent` hook forces retry | Stop-hook test gate |
| Done-ness | Model stops + optional Stop-hook/`/goal` | Model stops + stop-hook | No-tool-call = done | Model-judged | Model says done | Well-formed edit lints/tests clean or cap 3 | Critic/goal-judge + Stop-hook veto | Goals audit, evidence-based | Model stops, bounded | Model stops or Stop-hook gate |
| Structured-output validation | `final_output` JSON-Schema (workflows) | recipe `json_schema` | `submit_and_exit` | — | — | — | FinishTool | — | — | — |

## Where BioRouter was ahead

- **Background shell jobs are genuinely well-built and match the best.** The
  long-running review documents that `background=true` spawns into its own
  process group, a supervisor task records the **OS exit code as source of
  truth** (never scraped from log text), output is captured with a **per-job read
  cursor** so `shell_output` returns only new bytes (400KB cap), and `shell_wait`
  parks race-free on a `watch` channel with a bounded timeout that does *not* kill
  on expiry. The review explicitly notes this mirrors Claude Code and Codex. This
  is ahead of Goose (which only documents the pattern), Pi (no background
  bash at all — tmux is the workaround), Aider (no background manager), and
  OpenCode (background subagents still experimental with no intermediate output
  streaming). Only Claude Code and Codex CLI clearly match it.

- **Fork-bomb-guarded subagents with a global semaphore + in-flight ceiling.**
  Per the long-running review, a global `SUBAGENT_SEMAPHORE` (default 8) throttles
  concurrency and `SUBAGENT_INFLIGHT` (default 64) hard-refuses new spawns. Most
  competitors cap recursion (non-recursive subagents) but few document a global
  concurrent + queued ceiling; OpenHands caps `max_children=5` per parent but not
  globally.

- **Tool-result validation before persistence.** Per the core-loop review, every
  tool result is round-trip validated (`call_tool_result::validate`) before being
  stored, so a malformed payload cannot corrupt the persisted conversation. No
  other report describes a persistence-integrity guard of this kind.

- **Soft interrupts inject mid-turn without cancel-and-resend.** Per the core-loop
  and server-flow reviews, a user message can be drained at a safe loop boundary
  mid-turn. Pi has queued steering messages; most others discard in-flight work on
  interrupt. (Caveat: the `/interrupt` route is documented as orphaned/unwired in
  the desktop client — a server-flow review gap.)

- **Resource-aware scheduler deferral.** Per the long-running review, scheduled
  firings defer while rate-limited or while a user is interacting — a genuinely
  good property for a shared-provider research agent that competitors' cron
  systems (Cline, Goose, Codex Cloud) don't document.

## Where BioRouter was behind

> **Warning.** Several findings below were fixed after this review — see the
> status header. They are preserved as the record of what the review found.

BioRouter's execution story had three conspicuous holes: **no checkpoints/undo**,
**no automatic post-edit verification**, and **no git integration of agent
edits**. Each subsection below names the best-in-class mechanism the review
proposed reimplementing.

### Checkpoints and undo

*Best: Cline, then OpenCode and Gemini CLI.* BioRouter has *nothing* here (the
internal reviews describe SQLite session persistence but no file-state
snapshot/rewind). [Cline](../../../research/coding-agent-landscape/cline.md)
commits workspace state to a **shadow Git repo in extension storage** after every
tool use, keyed by a 13-char hash of the workspace path, leaving the user's real
`.git` untouched; it temporarily renames nested `.git`→`.git_disabled` to dodge
submodule conflicts, excludes `node_modules/`/`dist/`/binaries, and offers **three
restore axes** — Restore Files (code only), Restore Task Only (messages only),
Restore Both — each linked to a `ClineMessage` by timestamp.
[OpenCode](../../../research/coding-agent-landscape/opencode.md) is the safest
variant: it snapshots the worktree before/after **each model step** into a
**private Git object database in its own data dir** that creates no commits, moves
no branches, and touches no index; `/undo` restores changed files (deleting files
that didn't previously exist) and hides later messages, `/redo` reverses it,
exposed as `session.revert()`/`unrevert()`.
[Gemini CLI](../../../research/coding-agent-landscape/gemini-cli.md) commits to a
**shadow git repo in the home dir** before any file-modifying tool, persists
conversation + the pending tool call as JSON, and `/rewind` reverts files + chat +
**re-proposes the original tool call**, working across compression points. Any of
these is directly portable to BioRouter's Rust core; the shadow/private-repo
approach avoids polluting the scientist's git history.

### Post-edit verification loop

*Best: Claude Code and OpenCode (LSP), Aider (lint/test reflection).* The
verification review is blunt: `text_editor` writes never trigger diagnostics,
`analyze` (tree-sitter) is a **manual** tool, and an `LSP` tool "is even listed as
available but is not part of the developer extension's edit path." Claude Code's
`LSP` tool **automatically reports type errors/warnings after each edit**, giving
a built-in edit→check→fix micro-loop; OpenCode runs **25+ auto-detected LSP
servers** that feed diagnostics back after edits plus **auto-formatters
(rustfmt/gofmt/prettier) after every write**.
[Aider](../../../research/coding-agent-landscape/aider.md) is the cleanest closed
loop: after applying edits it **lints exactly the edited files**, and on failure
sets `reflected_message = lint_errors` so the failure re-enters the loop (same
channel for malformed edits, lint, tests, and missing-file mentions), hard-capped
at `max_reflections=3`. `--auto-test` does the same with the test command. To
reimplement in BioRouter: wire the existing `analyze`/`LSP` capability to fire
automatically on successful `text_editor` writes and feed diagnostics back as an
agent-visible message, bounded by a small reflection cap.

### Git integration of agent edits

*Best: Aider.* Per
[the Aider report](../../../research/coding-agent-landscape/aider.md):
**auto-commit per edit** with a weak-model-generated Conventional-Commit message,
a **"dirty commit"** first isolating pre-existing uncommitted changes, and
`(aider)` author/committer attribution so `git log` cleanly shows agent vs human
work; `/undo` reverts the last aider commit. Every edit becomes a natural,
bisectable checkpoint with no bespoke store. BioRouter is already described as
git-aware in the Knowledge feature but has no equivalent for agent code edits.

### Enforced test-after-edit and done-ness gate in interactive chat

*Best: Codex CLI (evidence audit), OpenHands (critic + goal-judge), Claude Code
(Stop-hook).* The verification review's "No enforced verification in interactive
chat" is the **biggest gap** — the only hard gate (`execute_success_checks`) is
workflow-only, single-variant (`Shell`), and on failure **discards all progress**
by resetting to initial messages rather than iterating on the diff.
[Codex CLI](../../../research/coding-agent-landscape/codex-cli.md) re-injects a
`goals/continuation.md` **completion audit** each turn that treats completion as
*unproven* and demands requirement-by-requirement evidence (files, command
output, test results, rendered artifacts), explicitly forbidding a
"narrower/easier-to-test" substitute.
[OpenHands](../../../research/coding-agent-landscape/openhands.md) has a
`CriticMixin` driving iterative refinement plus a `judge_goal` LLM returning a
calibrated 0–1 `complete` score and a `missing` string; a **Stop hook can veto
stopping** and flip status back to RUNNING. Claude Code's Stop-hook pattern runs
the test suite and blocks the agent from ending until green. BioRouter already
has the `/goal` Stop-hook + LLM-judge primitive (per the verification review) —
the gap is making it a default, test-backed gate rather than an opt-in user
choice.

### Per-tool timeout and structured error taxonomy

Per the core-loop review, timeouts are **per-extension (300s) only** — no per-tool
budget, no "this tool is taking a while" signal, and one slow tool blocks the
whole turn. Claude Code (Bash 2-min default / 10-min ceiling) and Codex
(`awaiter` 1h background poll with exponential backoff) bound individual calls.
Tool errors are unstructured `is_error` bools (verification review, gap #4);
OpenHands and Gemini CLI wrap failures into typed `functionResponse:{error}` the
model self-corrects on.

### Process tracking and orphan reaping

Per the long-running review: three disjoint in-memory registries,
`kill_on_drop(false)`, and **no PID-file / parent-death reaping** — a daemon crash
orphans process groups forever, even though the llama.cpp sidecar in the same repo
already implements `run/<ppid>.pid` reaping. Nothing to steal from a competitor
here; the fix (reuse the sidecar pattern) is internal. Also: a `shell_list`
surface exists but is dead-coded, so the agent cannot enumerate what it started
(long-running review, gap #3).

### Subagent results are lossy and fully blocking

Per the long-running review, gaps #4–#5: default `summary=true` returns only the
last text message ("No text content in last message" if the child ends on a tool
call), and the parent tool call blocks entirely — no async handle/poll. OpenCode's
background subagents (`task(background=true)` + `task_status`) and Codex's
`wait_agent`/`resume_agent` are the async-handle models to emulate.

## Best-in-class and worst-in-class per aspect

- **Tool dispatch model.** *Best (robustness across models):* **Aider**'s
  plain-text SEARCH/REPLACE formats parsed locally with parse-and-reflect — works
  on any model including weak/local ones, no function-calling dependency. *Best
  (parallel safety):* **Codex CLI**'s read/write-lock gating, which lets
  parallel-safe tools run concurrently while any mutating tool takes an exclusive
  lock. *Worst:* **BioRouter** — the core-loop review's gap #8 flags **unbounded
  `select_all` parallelism with no concurrency cap and no cross-tool isolation**,
  so concurrent edits to the same file race with no ordering guarantee. Gemini
  CLI's scheduler (force `update_topic` sequential, `wait_for_previous` opt-in,
  "only execute if ALL active calls are ready") and Codex's lock are both safer.

- **Oversized tool output.** *Best:* **Claude Code / Gemini CLI** — cap output,
  spill the rest to a file, and hand the model a **path + head preview** to
  grep/read on demand (lossless, bounded). *Worst:* **BioRouter** — core-loop
  review, gap #4: a 200,000-**character** per-content-item threshold (~50k tokens,
  so several sub-threshold items still overflow), dumps to
  `std::env::temp_dir()` **with no head/tail preview and no line-count summary**,
  and assumes tools can reach a path outside the session sandbox. OpenCode's
  "prune tool bodies before summarizing, protect last 2 turns, never prune skill
  outputs" is the most surgical.

- **Background processes.** *Best:* **Claude Code** (background Bash + the Monitor
  tool that feeds each output line back mid-turn without pausing) and **Codex**
  (dedicated `awaiter` sub-agent with exponential backoff). **BioRouter** is a
  strong second — its background job implementation is well-reviewed — but is held
  back by the tracking/orphan gap. *Worst:* **Pi** (synchronous bash only, tmux by
  hand) and **Aider** (no background manager).

- **Subagents.** *Best:* **Claude Code** (background-by-default, isolated context,
  summary-only return — the biggest context-hygiene win) and **Codex** (spawn/wait/
  resume/close + CSV fan-out). *Worst:* **Pi / Aider** (none by design). BioRouter
  sits mid-pack: bounded and safe but blocking and lossy.

- **Checkpoints / undo.** *Best:* **Cline** (three restore axes) and **OpenCode**
  (private git-object-DB, cleanest isolation). *Worst:* **BioRouter and
  Goose**: neither has any file snapshot or rewind; this is the single starkest
  deficit, and every external report that covers it flags the absence
  explicitly.

- **Git integration of edits.** *Best:* **Aider** (auto-commit per edit with
  attribution + dirty-commit isolation). *Worst:* **BioRouter, Pi** (no edit-level
  git integration documented).

- **Post-edit verification.** *Best:* **Claude Code** and **OpenCode** (automatic
  LSP diagnostics; OpenCode adds auto-formatters), with **Aider** best for the
  test-then-reflect closed loop. *Worst:* **Pi** (none by philosophy) and
  **BioRouter** (LSP/`analyze` present but not wired into the edit path —
  arguably worse than Pi because the capability exists unused).

- **Done-ness / enforced gate.** *Best:* **Codex CLI** (evidence-based completion
  audit) and **OpenHands** (critic + calibrated goal-judge). *Worst:* **Pi**
  ("runs until the agent says it's done," no gate) and **BioRouter** for
  interactive chat (per the verification review: "done is whatever the model
  decides," enforced verification is workflow-only). BioRouter's `/goal` judge is
  a good primitive that isn't a default.

## Implications and where they landed

The seven items below were the chapter's own recommendations. All of them were
consolidated into [the improvement proposals register](../improvement-proposals.md),
which is the authoritative list — read this section as the argument behind the
proposals, not as an open work queue. The register's BR-NN number is noted where
the mapping is one-to-one.

1. **Add file checkpoints + `/rewind` — the highest-value gap** (became
   **BR-43**). BioRouter, like Goose, has no undo at all, and its
   daemon/SQLite architecture is well-suited to OpenCode's model: snapshot the
   worktree before/after each model step (or each edit) into a **private git
   object DB in the app data dir** — no commits, no branch moves, no touching the
   user's `.git` — and expose Cline-style three-axis restore (files /
   conversation / both). This is the single biggest safety-net difference between
   BioRouter and current-generation agents.

2. **Wire the existing LSP/`analyze` capability into the edit path** (became
   **BR-47**). The pieces exist (the verification review notes `LSP` is listed and
   `analyze` is implemented) but nothing fires them after a `text_editor` write.
   Auto-run diagnostics on successful edits and feed failures back as an
   agent-visible message through a single bounded reflection channel (Aider's
   model), giving a real edit→check→fix loop for the R/Python/Rust research code
   BioRouter targets.

3. **Make the `/goal` Stop-hook + a shell success-check a default done-ness gate,
   not a workflow-only feature.** Reuse the mature `/goal` LLM-judge primitive and
   the `execute_success_checks` machinery, but (a) allow it in interactive chat,
   (b) add non-`Shell` check variants (file-exists, output-contains, JSON-schema),
   and (c) on failure surface *what failed* and iterate on the diff instead of
   resetting to initial messages.

4. **Add per-tool timeouts, a concurrency cap on parallel dispatch, and mutating-
   tool serialization.** Bound individual slow tools below the 300s extension
   ceiling, cap the unbounded `select_all` fan-out, and take Codex's exclusive
   write-lock for mutating tools so concurrent same-file edits can't race.

5. **Fix background-process lifecycle: reuse the sidecar's `run/<ppid>.pid`
   reaping, reset `currently_running` on scheduler reload, and surface the
   dead-coded `shell_list`.** These are internal fixes (long-running review, gaps
   #1–#3) that close real crash-orphan and stuck-job bugs without borrowing from
   any competitor.

6. **Improve oversized-tool-output handling to a head-preview + in-sandbox file**
   (became **BR-6**). Replace the raw temp-dir dump with a bounded head/tail
   preview plus a line-count summary and a handle the shell/file tools can
   actually reach (Claude Code / Gemini CLI model), and consider token-aware
   rather than character-count thresholds.

7. **Give subagents an async handle and a structured result envelope.** Offer a
   spawn→poll model (OpenCode `task_status` / Codex `wait_agent`) and return a
   typed status/error envelope so a subagent that ends on a tool call doesn't yield
   "No text content in last message."

## Related documentation

- [Core loop and tool dispatch](../subsystem-reviews/core-loop-and-tool-dispatch.md) — the internal review behind the dispatch, parallelism and oversized-output findings.
- [Long-running tasks and scheduling](../subsystem-reviews/long-running-tasks-and-scheduling.md) — the internal review behind the background-jobs, subagent and orphan-reaping findings.
- [Self-verification and done-ness](../subsystem-reviews/self-verification-and-doneness.md) — the internal review this chapter quotes on post-edit verification and the enforced-gate gap.
- [Shadow-git checkpoints design](../../../agent-loop/designs/shadow-git-checkpoints.md) — the design that answered this chapter's top recommendation, and the current truth on checkpoints.
- [The improvement proposals register](../improvement-proposals.md) — BR-1…BR-67, the consolidated list this chapter's implications fed into.
