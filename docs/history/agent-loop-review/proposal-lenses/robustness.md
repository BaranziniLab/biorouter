# Improvement proposals — Robustness and safety

> **What this is.** The robustness lens of the BioRouter agentic-loop improvement brainstorm of
> 2026-07-12: 50 proposals on loop and stuck detection, error-streak handling, checkpoints and undo
> of agent edits, crash/restart survival, permission and guardrail gaps, hook event coverage,
> sandboxing, and conversation-invariant edge cases (compaction boundaries, cancellation,
> tool_call/result pairing).
> **Status:** Historical record — this lens was merged into the master list
> ([Master improvement proposals](../improvement-proposals.md)) as part of BR-1 … BR-67, and that
> merged programme was then implemented. Flagship items from this lens shipped as **BR-43**
> (shadow-git checkpoints), **BR-29** and **BR-30** (staged repetition stop, oscillation detection),
> **BR-33** (single-turn lock), **BR-19** (hook rewrite and blocking) and **BR-46** (Anthropic
> `finish_reason`). Treat this file as the record of the reasoning, not as an open work queue.
> **Audience:** developers working on the agent loop, and maintainers tracing why a BR-numbered
> change was made.
> **Identifier key.** `P-NN` numbers are **local to this file**. Each of the three lens files
> restarts its numbering at `P-1`, so `P-12` here is a different proposal from `P-12` in the
> performance or UX lens; the master list disambiguates them as `robustness P-12`. `BR-NN` numbers
> are the merged master-list ids, indexed in
> [Master improvement proposals](../improvement-proposals.md).

This is one of three lens files that read the same evidence base through one concern each — this one
through robustness and safety. Each proposal is self-contained; several are cheap quick wins and a
few are ambitious redesigns. Every proposal carries the same seven fields (Problem, Proposal,
Inspired by, Affected code, Impact, Effort, Risk) and cites the review that establishes the gap.
Effort is graded S (hours) / M (days) / L (weeks).

The proposals are ordered by theme rather than priority; the `##` headings below name those themes,
and the [highest-impact proposals](#highest-impact-proposals-in-this-lens) section at the end
collects the items this lens itself rated high impact.

## Evidence base

The short paths cited throughout each proposal refer to the reviews below, which have since moved to
these locations.

| Cited as | Document |
|---|---|
| `internal/core-loop.md` | [Core loop and tool dispatch](../subsystem-reviews/core-loop-and-tool-dispatch.md) |
| `internal/loop-detection.md` | [Loop and stuck detection](../subsystem-reviews/loop-and-stuck-detection.md) |
| `internal/hooks.md` | [Hooks system](../subsystem-reviews/hooks-system.md) |
| `internal/guardrails-permissions.md` | [Guardrails and permissions](../subsystem-reviews/guardrails-and-permissions.md) |
| `internal/compaction.md` | [Compaction and context management](../subsystem-reviews/compaction-and-context-management.md) |
| `internal/long-running.md` | [Long-running tasks and scheduling](../subsystem-reviews/long-running-tasks-and-scheduling.md) |
| `internal/state-awareness.md` | [State awareness and version control](../subsystem-reviews/state-awareness-and-version-control.md) |
| `internal/verification.md` | [Self-verification and done-ness](../subsystem-reviews/self-verification-and-doneness.md) |
| `internal/server-flow.md` | [Server reply flow and session lifecycle](../subsystem-reviews/server-reply-flow-and-session-lifecycle.md) |
| `internal/context-injection.md` | [Context injection and system prompt](../subsystem-reviews/context-injection-and-system-prompt.md) |
| `compare/safety.md` | [Safety and guardrails, compared](../competitive-comparison/safety-and-guardrails.md) |
| `compare/context.md` | [Context and prompts, compared](../competitive-comparison/context-and-prompts.md) |

> **Terms used below.** **SOTA** is state-of-the-art, the shorthand this lens uses for the
> comparator agents surveyed in the competitive-comparison chapters. **HITL** is human-in-the-loop
> tool approval: the agent pauses before a tool call and waits for a person to allow or deny it.
> **PII/PHI** are personally identifiable information and protected health information, the two
> classes the on-device masker in `guardrails/pii.rs` targets. **BRSDK** is the BioRouter App SDK —
> the client library plus the server-side runner that a generated Agent Drafter app talks to.

## Contents

- [Loop and stuck detection](#loop-and-stuck-detection) — P-1 … P-9
- [Error streaks and provider-failure recovery](#error-streaks-and-provider-failure-recovery) — P-10 … P-11
- [Checkpoints and undo of agent edits](#checkpoints-and-undo-of-agent-edits) — P-12 … P-13
- [Crash and restart survival](#crash-and-restart-survival) — P-14 … P-18
- [Permissions and command safety](#permissions-and-command-safety) — P-19 … P-25
- [Hook coverage and capability](#hook-coverage-and-capability) — P-26 … P-31
- [Sandboxing and process isolation](#sandboxing-and-process-isolation) — P-32
- [Server flow, concurrency and cancellation](#server-flow-concurrency-and-cancellation) — P-33 … P-38
- [Conversation invariants and compaction](#conversation-invariants-and-compaction) — P-39 … P-45
- [Tool execution safety and enforced verification](#tool-execution-safety-and-enforced-verification) — P-46 … P-48
- [Observability and institutional policy](#observability-and-institutional-policy) — P-49 … P-50
- [Highest-impact proposals in this lens](#highest-impact-proposals-in-this-lens)

## Proposals also raised by another lens

Where the same idea surfaced in more than one lens, the master list kept the richer writeup and
tagged the overlap. Several entries below restate a sibling lens entry almost verbatim; these are
this file's cross-lens duplicates.

| This file | Also raised as |
|---|---|
| P-1, P-2, P-4 (repetition reason, staged stop, oscillation detection) | `ux P-7` |
| P-10 (mistake-streak handling) | `ux P-27` |
| P-11 (loop-level retry for streaming errors) | `performance P-34`, `ux P-32` |
| P-12 (shadow-git checkpoints and undo) | `ux P-1` |
| P-14, P-15 (reap orphans, reconcile stuck jobs) | `performance P-48` |
| P-16 (`shell_list` for background jobs) | `ux P-9` |
| P-17 (persist goal state) | `ux P-3` |
| P-19 (read-only auto-approve and risk grading) | `performance P-39`, `ux P-6` |
| P-24 (per-directory / per-prefix permission scoping) | `ux P-5` |
| P-26, P-27 (PostToolUse blocking, PreToolUse `updated_input`) | `ux P-33` |
| P-33 (single-turn-per-session lock) | `performance P-45`, `ux P-16` |
| P-34, P-35 (confirmation TTL, cancellation-aware wait) | `performance P-47`, `ux P-11`, `ux P-12` |
| P-38 (idempotency / resume token on `/reply`) | `performance P-46` |
| P-40 (Anthropic `finish_reason`) | `ux P-25` |
| P-42 (head/tail-truncate an over-window message) | `performance P-7`, `ux P-21` |
| P-45 (progressive context-overflow fallback) | `performance P-10` |
| P-46 (bound tool parallelism) | `performance P-32` |
| P-47 (post-edit diagnostics feedback loop) | `ux P-17` |
| P-48 (wire the `structured_output` validation loop) | `ux P-24` |

---

## Loop and stuck detection

### P-1: Surface the repetition-limit reason to the model
- **Problem:** On a `RepetitionInspector` denial the model receives the generic `DECLINED_RESPONSE` ("The user has declined to run this tool…"), not the true "exceeded maximum repetitions" reason — only *hook* denials get their reason forwarded (`internal/loop-detection.md` #2, `agent.rs:757-766`, `tool_execution.rs:38-40`). This is actively misleading: the model thinks the user refused and may abandon a tool it legitimately needs or hallucinate a refusal.
- **Proposal:** Forward the `REP-001` reason into the tool-result text with corrective guidance ("You have called this tool identically N times; change approach or stop"), the same way hook reasons are appended.
- **Inspired by:** Cline (recoverable error + injected guidance), Gemini CLI.
- **Affected code:** `crates/biorouter/src/agents/agent.rs:748-781`, `agents/tool_execution.rs:38-40`, `tool_monitor.rs:151-159`.
- **Impact:** medium — removes a false signal that degrades recovery on every loop trigger.
- **Effort:** S
- **Risk:** Low; only changes the text fed back on an already-denied call.

### P-2: Staged soft-then-hard repetition stop
- **Problem:** Repetition detection is a single hard deny at the 4th identical call, with no soft nudge first; a legitimate 4th retry is silently blocked (`internal/loop-detection.md` #1, `compare/safety.md` "Staged/soft-then-hard loop stop: no (single deny)").
- **Proposal:** Emit a non-blocking soft warning at 3 identical calls (injected as guidance), only escalate to `Deny` at 5. Cheapest single upgrade toward SOTA loop handling.
- **Inspired by:** Cline / OpenCode (3 warn / 5 stop `doom_loop` gate), Gemini CLI 3-layer.
- **Affected code:** `crates/biorouter/src/tool_monitor.rs:107-164`, `tool_inspection.rs`.
- **Impact:** medium — fewer false stops, clearer escalation.
- **Effort:** S
- **Risk:** Low; raising the hard threshold slightly loosens the loop guard, offset by the soft warning.

### P-3: Consolidate the two RepetitionInspector implementations
- **Problem:** `check_tool_call` (stateful, mutates `last_call`/`repeat_count`) is only exercised by unit tests; production runs the stateless `inspect`, so `last_call`/`repeat_count`/`call_counts`/`reset()` are dead in prod. A future fix or tuning can land in the untested-in-prod path (`internal/loop-detection.md` #3/#10).
- **Proposal:** Delete `check_tool_call` (or make `inspect` delegate to a single shared core) and re-point the tests at the production path. Also delete or wire `RetryManager::with_repetition_inspector`, which is never called.
- **Inspired by:** novel (dead-code hygiene).
- **Affected code:** `crates/biorouter/src/tool_monitor.rs:59-88`, `agents/retry.rs:63-83`, `tests/repetition_inspector_tests.rs`.
- **Impact:** low — correctness/maintainability, prevents a latent trap.
- **Effort:** S
- **Risk:** Low.

### P-4: Semantic / near-duplicate & oscillation loop detection
- **Problem:** `matches` requires byte-exact JSON and counts only *consecutive* calls, so a one-char arg change, an `A/B/A/B` oscillation, or a semantically-identical-but-textually-different call all bypass it (`internal/loop-detection.md` #1, `internal/state-awareness.md` #8). OpenHands detects alternating `[A,B,A,B]` (N=4) and action-error repetition (N=3); BioRouter detects neither.
- **Proposal:** Add heuristics to the inspector over the last ~20 events after the last user message: normalized-arg similarity (ignore ids/whitespace), alternating-pattern detection, and repeated action-*error* detection.
- **Inspired by:** OpenHands `StuckDetector` (5 heuristics), Gemini CLI.
- **Affected code:** `crates/biorouter/src/tool_monitor.rs`, `tool_inspection.rs`.
- **Impact:** high — catches the loop classes that actually occur in practice.
- **Effort:** M
- **Risk:** Medium; heuristics can false-positive — gate behind soft warnings (P-2) first.

### P-5: Repeated-failing-result / no-progress detector
- **Problem:** The inspector never looks at tool *results*; repeated identical error messages ("no such file" over and over), or a command that keeps failing the same way, are invisible. There is no "no file changed / no new information in N turns" detector outside `/goal` (`internal/loop-detection.md` #1, `internal/verification.md` #7).
- **Proposal:** Hash tool-result content (or its error signature) and track repeats; when the same failing outcome recurs N times, inject a "you are not making progress; change approach or ask the user" nudge and, on persistence, block.
- **Inspired by:** Gemini CLI content-chant detector, OpenHands action-error.
- **Affected code:** `crates/biorouter/src/tool_monitor.rs`, `agents/agent.rs` (result collection at `:1792-1843`).
- **Impact:** high — closes the biggest honest loop-detection gap.
- **Effort:** M
- **Risk:** Medium; needs careful result-normalization to avoid nuisance nudges.

### P-6: Bring `/goal` stall detection to ordinary chat
- **Problem:** The genuinely good progress-stall logic (Jaccard `reason_similarity`, `GOAL_STALL_LIMIT`, non-resetting iteration cap) lives only in the `/goal` Stop-hook loop and never runs for ordinary chat, which is where most stuck loops happen (`internal/loop-detection.md` #9, `internal/state-awareness.md` #8, `goal.rs:301-320`).
- **Proposal:** Factor the stall detector out of `goal.rs` and run a lightweight version at Stop-time for all sessions (a background "are you looping?" check on the transcript tail), not just when a goal is set.
- **Inspired by:** Gemini CLI periodic LLM loop check, BioRouter's own goal loop.
- **Affected code:** `crates/biorouter/src/agents/goal.rs:121-133,301-320`, `agents/agent.rs:2120-2233`.
- **Impact:** high — extends a mature primitive to the common case.
- **Effort:** M
- **Risk:** Medium; an always-on LLM stall check adds cost/latency — make it periodic (e.g. after turn 30).

### P-7: Absolute per-tool call ceiling
- **Problem:** `call_counts` tracks per-tool totals but is never read for any decision; a tool called hundreds of times with ever-changing args trips nothing except the loose 100-turn cap (`internal/loop-detection.md` #5, `tool_monitor.rs:46`).
- **Proposal:** Add a configurable absolute ceiling (e.g. a tool run > K times per reply requires approval / is denied), reading the already-tracked `call_counts`.
- **Inspired by:** Codex CLI (goal token budget), novel.
- **Affected code:** `crates/biorouter/src/tool_monitor.rs:46,61-65`.
- **Impact:** medium — a backstop the exact-duplicate guard misses.
- **Effort:** S
- **Risk:** Low; ceiling set high enough not to bite normal work.

### P-8: Global wall-clock / token / dollar budget per reply
- **Problem:** Only the 100-turn iteration count bounds a reply; 429 backoff (~2 min/call) compounds inside it, so a throttled or pathological session can run far longer than a user expects with no wall-clock guard (`internal/loop-detection.md` #6, `internal/core-loop.md`, `compare/safety.md` "Budget cap: no").
- **Proposal:** Track cumulative wall-clock, tokens, and (if pricing known) dollars per reply; on exceeding a configurable budget, stop gracefully with a "budget reached, continue?" message like the turn cap. Re-inject `remaining_tokens` so the model wraps up.
- **Inspired by:** Codex CLI (token budget + `budget_limit.md`), OpenHands (`max_budget_per_run` → `MaxBudgetReached`).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:1556-1583` (loop), `agents/types.rs` (SessionConfig).
- **Impact:** high — bounds cost/time, not just iterations.
- **Effort:** M
- **Risk:** Low; additive soft stop.

### P-9: Count tool calls, not just loop iterations, in the turn cap
- **Problem:** `max_turns` counts loop iterations (provider round-trips); with parallel tool fan-out per iteration, "100 actions" is a loose bound on real work/side-effects (`internal/loop-detection.md` #8, `agent.rs:1792-1843`).
- **Proposal:** Also track a per-reply tool-call counter and apply a (higher) cap on it, so a few iterations each firing dozens of parallel writes are bounded.
- **Inspired by:** novel.
- **Affected code:** `crates/biorouter/src/agents/agent.rs:1571-1583`.
- **Impact:** low/medium.
- **Effort:** S
- **Risk:** Low.

## Error streaks and provider-failure recovery

### P-10: Mistake-streak / recoverable-failure handling
- **Problem:** There is no counter for consecutive `api_error` / `invalid_tool_call` / `tool_execution_failed`; a non-context provider error just ends the turn with a "please retry" string, and there is no "one more chance with a hint" pattern (`compare/safety.md` behind #6, `internal/core-loop.md` #5, `internal/state-awareness.md` #7).
- **Proposal:** Add a `MistakeTracker` over the last N tool/provider outcomes: below a cap emit a recoverable error and continue; at the cap inject a recovery notice (resetting the counter) or stop with preserved state.
- **Inspired by:** Cline `MistakeTracker` (best-in-class), Aider `reflected_message` (cap 3).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:2020-2028`, new module.
- **Impact:** high — strictly better than a hard end-turn on transient failure.
- **Effort:** M
- **Risk:** Medium; must not mask genuinely fatal errors.

### P-11: Loop-level retry for streaming/provider errors
- **Problem:** The streaming path is not wrapped in `ProviderRetry`; a mid-stream decode error or any non-context `ProviderError` ends the turn and pushes the retry decision to the user (`internal/core-loop.md` #5, `anthropic.rs:273-313`).
- **Proposal:** Wrap the streaming call in bounded backoff for transient errors (rate limit / 5xx / decode), resuming or restarting the turn a small number of times before surfacing to the user.
- **Inspired by:** BioRouter's own non-streaming `with_retry`, all comparators.
- **Affected code:** `crates/biorouter/src/providers/anthropic.rs:273-313`, `providers/retry.rs`, `agents/agent.rs:2020-2028`.
- **Impact:** medium — fewer user-visible failures on long streams.
- **Effort:** M
- **Risk:** Medium; must record partial usage and avoid double-billing on restart.

## Checkpoints and undo of agent edits

### P-12: Shadow-git checkpoint & task-level undo of agent edits
- **Problem:** There is no git checkpointing, no shadow git, no session-level undo. The only rollback is `text_editor`'s in-memory, per-file, per-process LIFO — it dies with the process, misses shell/`write_file`/other-extension writes, and offers no "revert the whole task" (`internal/state-awareness.md` #2/#3, `compare/safety.md` behind #9 — called the single biggest gap in the Claude Code external review).
- **Proposal:** Add a shadow-repo (private git object DB or `git stash`-style snapshot) that checkpoints the workspace before each risky turn/tool batch, with a "revert this turn / this task" surface. Covers all writers, not just `text_editor`.
- **Inspired by:** Cline shadow-git, OpenCode private git-object DB, Gemini/Claude Code shadow-repo + rewind, Aider commit-per-edit.
- **Affected code:** new module in `crates/biorouter/src/`, hooks in `agents/agent.rs` tool dispatch; `git2` already a dependency.
- **Impact:** high — the recovery mechanism that makes aggressive autonomy tolerable.
- **Effort:** L
- **Risk:** Medium; must be gitignore-aware, not pollute the user's repo, and handle large/binary files.

### P-13: Persist and extend `text_editor` undo history
- **Problem:** The `text_editor` undo stack is `Arc<Mutex<HashMap<PathBuf,Vec<String>>>>` created fresh per `DeveloperServer` process and never persisted; it only covers `text_editor` edits (`internal/state-awareness.md` #2/#3, `text_editor.rs:1052-1106`).
- **Proposal:** As an incremental step toward P-12, persist per-path undo history to disk (or the session DB) and record shell-redirect / `write_file` mutations so `undo_edit` covers them.
- **Inspired by:** Aider, Cline.
- **Affected code:** `crates/biorouter-mcp/src/developer/{text_editor.rs,rmcp_developer.rs:698}`.
- **Impact:** medium.
- **Effort:** M
- **Risk:** Low/medium.

## Crash and restart survival

### P-14: Reap orphaned background shell jobs across restarts
- **Problem:** Background shell jobs set `kill_on_drop(false)` and live in an in-memory per-`DeveloperServer` registry with no PID-file/parent-death reaping; a daemon crash orphans whole process groups forever with no way to discover or kill them — even though the llama.cpp sidecar already implements exactly this reaping (`internal/long-running.md` #1, `background.rs:119`, `llamacpp_sidecar.rs:833-936`).
- **Proposal:** Reuse the sidecar's `run/<ppid>.pid` pattern: record background job PIDs to a run-dir file and sweep/kill children of dead parents on `DeveloperServer` start.
- **Inspired by:** BioRouter's own llama.cpp sidecar, Claude Code / Codex.
- **Affected code:** `crates/biorouter-mcp/src/developer/background.rs`, `rmcp_developer.rs:704`.
- **Impact:** medium/high — prevents resource leaks and zombie processes.
- **Effort:** M
- **Risk:** Low; must only reap jobs it owns.

### P-15: Reconcile `currently_running` on scheduler load
- **Problem:** `load_jobs_from_storage` reinserts each job verbatim without resetting `currently_running`/`current_session_id`/`process_start_time`; a job mid-run at crash time reloads as running and is then *permanently skipped* by the overlap guard — a stuck-job bug on every crash (`internal/long-running.md` #2, `scheduler.rs:512-548,175-178`).
- **Proposal:** One-line reconcile on load: force `currently_running = false` (and clear the session id / start time) for every loaded job.
- **Inspired by:** novel (crash-recovery hygiene).
- **Affected code:** `crates/biorouter/src/scheduler.rs:512-548`.
- **Impact:** medium — fixes a silent permanent-skip after any crash.
- **Effort:** S
- **Risk:** Low; a job that was genuinely still running is rare and the process is gone anyway.

### P-16: `shell_list` tool for background jobs
- **Problem:** `job_id`s are ephemeral in-memory ints with no enumeration surface (`list()` exists but is `#[allow(dead_code)]`); if the agent forgets a `job_id` mid-session it cannot discover what it started (`internal/long-running.md` #3, `background.rs:251`).
- **Proposal:** Surface the existing `list()` as a `shell_list` tool returning job ids, commands, and statuses.
- **Inspired by:** Claude Code / Codex "list background tasks".
- **Affected code:** `crates/biorouter-mcp/src/developer/{background.rs:251,rmcp_developer.rs}`.
- **Impact:** low/medium.
- **Effort:** S
- **Risk:** Low.

### P-17: Persist goal state like todos
- **Problem:** Goal state is in-memory only (`GoalRegistry` on the `Agent`), so a daemon restart silently drops an active `/goal` while todos (in `extension_data`) survive — an inconsistency that confuses users (`internal/state-awareness.md` #3, `goal.rs:99-101`).
- **Proposal:** Persist `GoalState` into `session.extension_data` (versioned key like `todo.v0`) and reload it on resume.
- **Inspired by:** BioRouter's own todo persistence.
- **Affected code:** `crates/biorouter/src/agents/goal.rs:99-101`, `extension_data.rs`.
- **Impact:** medium.
- **Effort:** S
- **Risk:** Low.

### P-18: Recover or cleanly fail pending elicitations & in-flight runs on restart
- **Problem:** `ActionRequiredManager`'s pending oneshots are in-memory; a restart drops them and any parked tool call is lost with no user signal. Subagents and background jobs likewise do not resume (`internal/long-running.md` "What survives a daemon restart", #10).
- **Proposal:** Persist pending elicitations/approvals (RunState already models a paused approval and survives reconnects — extend the pattern) and, on startup, surface "this run was interrupted" instead of silently hanging. Make elicitation session-scoped rather than a process-wide singleton.
- **Inspired by:** BioRouter's own `run_state.rs`, OpenHands persisted event store.
- **Affected code:** `crates/biorouter/src/action_required_manager.rs:17-31`, `guardrails/run_state.rs`, `agents/mcp_client.rs:254-285`.
- **Impact:** medium.
- **Effort:** L
- **Risk:** Medium; concurrency and routing correctness.

## Permissions and command safety

### P-19: Wire read-only auto-approve and per-action risk grading
- **Problem:** `PermissionInspector`'s `readonly_tools`/`regular_tools` sets are constructed empty with no setter, so the read-only short-circuit never fires and every non-user-configured tool requires approval; the LLM permission judge (`check_tool_permissions`) has zero callers — so `SmartApprove` is behaviorally identical to `Approve` (`internal/guardrails-permissions.md` #1/#2, `compare/safety.md` behind #2).
- **Proposal:** Populate `readonly_tools`/`regular_tools` from the extension manager's `read_only_hint` annotations (the comment says this was intended), and adopt OpenHands' per-action `security_risk` (LOW/MED/HIGH/UNKNOWN) + `ConfirmRisky(threshold, confirm_unknown)` shape rather than resurrecting the dead judge. Delete the unreachable `check_tool_permissions` path.
- **Inspired by:** OpenHands (`security_risk` + `ConfirmRisky`, read-only exempt, fail-safe HIGH), Claude Code auto-mode.
- **Affected code:** `crates/biorouter/src/permission/permission_inspector.rs:106-188`, `agent.rs:348-351`, `permission/permission_judge.rs`.
- **Impact:** high — makes the "smart" tier actually smart and stops over-prompting reads.
- **Effort:** M
- **Risk:** Medium; auto-approving reads is a real trust decision — gate on `read_only_hint` accuracy.

### P-20: Always-on non-bypassable catastrophic-command denylist
- **Problem:** Dangerous-command detection is off by default (`SECURITY_PROMPT_ENABLED=false`) and, even enabled, only ever *asks* (`should_ask_user: true`) — so in `Auto` mode a user gets no command screening at all, because Auto allows everything before the disabled scanner would matter (`internal/guardrails-permissions.md` #3, `security/mod.rs:35-41,133-142`).
- **Proposal:** Add a small always-on, non-bypassable hard-block list for a handful of catastrophic patterns (`rm -rf /`, disk-wipe `dd`, fork bombs) that fires even in `Auto` mode and cannot be disabled by config.
- **Inspired by:** Claude Code / OpenCode deny-by-default catastrophic rules.
- **Affected code:** `crates/biorouter/src/security/{mod.rs,patterns.rs,security_inspector.rs}`.
- **Impact:** high — closes the "Auto mode = zero screening" hole.
- **Effort:** S/M
- **Risk:** Medium; false positives block legitimate work — keep the list tiny and high-confidence.

### P-21: Replace regex command scanner with an auditable policy engine
- **Problem:** The 40+-entry regex table is trivially evadable (`r''m -rf`, `$(printf …)`, env-var indirection, a different tool wrapper) with no argv parsing or path canonicalization — a signature scanner presented as a security control (`internal/guardrails-permissions.md` #4, `compare/safety.md` behind #4).
- **Proposal:** Parse argv and canonicalize paths, and move rules into a declarative, testable policy (Codex `execpolicy` Starlark `prefix_rule` with self-tests + `host_executable` pinning, or Gemini's tiered TOML with an admin tier). Lives outside the binary as config.
- **Inspired by:** Codex `execpolicy` (best-in-class), Gemini CLI TOML policy engine, OpenCode wildcard last-match-wins.
- **Affected code:** `crates/biorouter/src/security/{patterns.rs,scanner.rs,mod.rs}`.
- **Impact:** high — real command governance for a lab/UCSF deployment.
- **Effort:** L
- **Risk:** Medium; new engine surface to get right.

### P-22: Scan tool *output* on the main loop (injection + PII)
- **Problem:** Guardrails (PII masking, `Block`, run_state HITL) run only on the Agent Drafter app socket; the CLI/GUI loop has no PII stage and never scans tool *output* — the classic prompt-injection vector for agents reading web/file content. `GuardrailStage::{ToolInput,ToolOutput,Output}` are declared but unused (`internal/guardrails-permissions.md` #6/#9, `compare/safety.md` behind #8).
- **Proposal:** Add a tool-*result* guardrail stage on the main loop that scans returned content for injection markers and PII/PHI (reusing the existing local `pii.rs`), masking or quarantining before it enters the model context.
- **Inspired by:** Claude Code protected paths, OpenHands, novel for the PII local-first angle.
- **Affected code:** `crates/biorouter/src/guardrails/{mod.rs:13-26,pii.rs}`, `agents/agent.rs` result path (`:981,1808`), `large_response_handler.rs`.
- **Impact:** high — the biggest injection surface is currently unguarded.
- **Effort:** M
- **Risk:** Medium; false-positive masking could hide real data — make it opt-in / mode-gated.

### P-23: Central secret-redaction boundary across all extensions
- **Problem:** `.biorouterignore` lives inside the Developer MCP server only, so any other extension (compute, files, third-party MCP, a different shell wrapper) that reads `.env`/`secrets.*` bypasses it; default patterns also miss `.pem`, `id_rsa`, `.aws/credentials` (`internal/guardrails-permissions.md` #7).
- **Proposal:** Move ignore/redaction enforcement to a central boundary (the tool-dispatch or extension-manager layer) applied to every read-side tool, and widen the default deny set.
- **Inspired by:** Claude Code protected paths.
- **Affected code:** `crates/biorouter-mcp/src/developer/rmcp_developer.rs:1670-1704` (extract), `crates/biorouter/src/agents/extension_manager.rs` dispatch.
- **Impact:** high — one bypassable boundary today.
- **Effort:** M
- **Risk:** Medium; must not break legitimate reads of config files.

### P-24: Per-directory / per-prefix permission scoping
- **Problem:** `ToolPermissionStore` keys `AlwaysAllow` on `blake3(tool_name + exact-JSON args)`, so "always allow `shell`" is either exact-args reuse or a blanket whitelist of *all* future `shell` invocations, including dangerous ones. No "allow reads under this dir but not writes" (`internal/guardrails-permissions.md` #8).
- **Proposal:** Add scoped permission grants (tool + command-prefix or path-glob + operation class) so a user can persist "allow `git` in this repo" without whitelisting arbitrary shell.
- **Inspired by:** OpenCode wildcard rules, Gemini tiered TOML, Claude Code allow/ask/deny rules.
- **Affected code:** `crates/biorouter/src/permission/{permission_store.rs:79-127,permission_inspector.rs}`.
- **Impact:** medium/high.
- **Effort:** M
- **Risk:** Medium; matching semantics must be conservative.

### P-25: Fix `unwrap()` panics in the permission store
- **Problem:** `ToolPermissionStore` calls `.unwrap()` on `tool_call` and will panic if a `ToolRequest` carries an `Err` tool_call; inspectors guard with `if let Ok`, but the store does not (`internal/guardrails-permissions.md` #10, `permission_store.rs:81,99,122`).
- **Proposal:** Return a deny/error on a malformed request instead of panicking (fail-closed, not crash).
- **Inspired by:** novel (defensive correctness).
- **Affected code:** `crates/biorouter/src/permission/permission_store.rs:81,99,122`.
- **Impact:** low — a crash-to-panic hardening.
- **Effort:** S
- **Risk:** Low.

## Hook coverage and capability

### P-26: Let PostToolUse hooks block
- **Problem:** PostToolUse / PostToolUseFailure are observe-only although the block decision is already computed — so "reject a write that fails lint" is impossible, diverging from Claude Code (`internal/hooks.md` #2, `compare/safety.md` "PostToolUse can block: no", `agent.rs:1845-1847`).
- **Proposal:** Honor the computed PostToolUse decision: on block, feed the reason back as a corrective tool result and keep the agent working.
- **Inspired by:** Claude Code, Cline, OpenHands, Codex, Gemini (all allow PostToolUse block).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:1845-1913`, `hooks/`.
- **Impact:** high — unlocks the most useful post-hoc guardrail.
- **Effort:** M
- **Risk:** Medium; a bad hook could wedge a turn — bound with the existing block-cap pattern.

### P-27: PreToolUse tool-input rewrite (`updated_input`)
- **Problem:** Hooks can only allow/deny/ask/inject; there is no rewrite path anywhere, so a hook cannot sandbox a path, redact a payload, or normalize a shell command (`internal/hooks.md` #7, `compare/safety.md` behind #1 — the single biggest capability gap).
- **Proposal:** Add Codex's `PreToolUseOutcome` shape: an optional `updated_input` applied to the tool args before dispatch (plus `should_block`/`block_reason`/`additional_contexts`).
- **Inspired by:** Codex CLI `updated_input` (cleanest), Gemini `hookSpecificOutput.tool_input`, Pi `event.input` mutation.
- **Affected code:** `crates/biorouter/src/hooks/{outcome.rs,inspector.rs:57-88}`, `agents/agent.rs` dispatch.
- **Impact:** high — turns hooks from a veto into a policy engine.
- **Effort:** M
- **Risk:** Medium; rewritten input bypasses re-validation — document and consider re-running inspectors.

### P-28: Stop silently dropping PreToolUse / PermissionRequest hook context
- **Problem:** `HookInspector` and `tool_execution` read only `aggregate.decision`, so `additionalContext`/`systemMessage` returned by PreToolUse or PermissionRequest hooks are silently discarded — a confusing, undocumented dead end (`internal/hooks.md` #1, `inspector.rs:62`, `tool_execution.rs:77`).
- **Proposal:** Consume and inject `additional_context`/`system_messages` from these events like the SessionStart/UserPromptSubmit path already does.
- **Inspired by:** Claude Code (injects PreToolUse additionalContext).
- **Affected code:** `crates/biorouter/src/hooks/inspector.rs:62`, `agents/tool_execution.rs:77`, `agent.rs:1411-1422`.
- **Impact:** medium — high-value, low-risk fix per the review.
- **Effort:** S
- **Risk:** Low.

### P-29: Return aggregates from `fire()` hook events
- **Problem:** Notification, SubagentStart/Stop, Pre/PostCompact spawn detached tasks and drop the `HookAggregate` entirely, so even a `systemMessage` is lost, there is no way to know a compaction/subagent hook ran, and fire-and-forget can outlive the turn and race shutdown (`internal/hooks.md` #3).
- **Proposal:** Await these hooks (or at least capture and surface their aggregate/errors), and join outstanding hook tasks at turn/shutdown boundaries.
- **Inspired by:** novel (observability + lifecycle correctness).
- **Affected code:** `crates/biorouter/src/hooks/mod.rs:258-271`, fire sites in `agents/`.
- **Impact:** medium.
- **Effort:** M
- **Risk:** Low/medium; awaiting adds latency to those lifecycle points.

### P-30: Output-size limits + untrusted framing on injected hook stdout
- **Problem:** Raw stdout (UserPromptSubmit/SessionStart) and `additionalContext` are injected verbatim with no truncation — a hook emitting megabytes silently bloats/blows the context, and it is a prompt-injection surface (a project hook's stdout lands as a hidden user message) (`internal/hooks.md` #5, `internal/context-injection.md` untrusted-file framing gap).
- **Proposal:** Cap injected hook output size (truncate with a marker) and wrap it in explicit data-not-instruction framing.
- **Inspired by:** Codex `project_doc_max_bytes`, Claude Code lower-trust project files.
- **Affected code:** `crates/biorouter/src/hooks/outcome.rs:180-186`, `agents/agent.rs:1413-1422`.
- **Impact:** medium.
- **Effort:** S
- **Risk:** Low.

### P-31: Matcher on tool_input content, not just tool name
- **Problem:** Hook matchers only see the tool name, so "only guard `rm -rf`" or "only writes under `/etc`" is impossible — every shell command must run the full guard script; the regex is also recompiled every call (`internal/hooks.md` #8, `matcher.rs:21`).
- **Proposal:** Extend matchers to optionally match on `tool_input` fields (e.g. a command/path regex), and cache compiled regexes.
- **Inspired by:** Gemini CLI args-regex rules, Claude Code.
- **Affected code:** `crates/biorouter/src/hooks/matcher.rs:10-28`.
- **Impact:** medium.
- **Effort:** M
- **Risk:** Low.

## Sandboxing and process isolation

### P-32: OS-level sandbox for tool execution
- **Problem:** BioRouter has no process isolation at all — its guardrail is permission gating, so autonomy is bounded by prompt compliance and the currently-off regex scanner, not the kernel (`compare/safety.md` behind #3, `internal/guardrails-permissions.md`).
- **Proposal:** Adopt Codex's two-axis model (what is technically possible via OS sandbox vs when to ask via approval): macOS Seatbelt `sandbox-exec -p` with writable-roots injected and network denied, Linux Landlock+seccomp+bubblewrap, escalate-to-approval on a sandbox denial rather than hard-fail.
- **Inspired by:** Codex CLI (best-in-class), OpenHands (Docker/VM), Gemini CLI, Claude Code Bash sandbox.
- **Affected code:** `crates/biorouter-mcp/src/developer/` shell exec, `crates/biorouter/src/security/`, spawn paths in `extension_manager.rs`.
- **Impact:** high — kernel-enforced bound on autonomy.
- **Effort:** L
- **Risk:** High; platform-specific, can break legitimate tool access — needs careful writable-root config.

## Server flow, concurrency and cancellation

### P-33: Server-enforced single-turn-per-session lock
- **Problem:** There is no server-side one-turn-per-session guard; two concurrent `/reply` calls for one `session_id` share one `Arc<Agent>`, one `confirmation_rx`, and one `soft_interrupts` vec, interleaving turns. Serialization is enforced only client-side (`internal/server-flow.md` gap #1 — "the single most important gap", `reply.rs:257`, `manager.rs:84-116`).
- **Proposal:** Hold a per-session turn lock/queue in the server; a second `/reply` either queues or is rejected with "turn in progress."
- **Inspired by:** state-of-the-art agents (per-session turn lock).
- **Affected code:** `crates/biorouter-server/src/routes/reply.rs:257`, `state.rs`, `execution/manager.rs`.
- **Impact:** high — prevents shared-state corruption from raced clicks / multi-window / CLI.
- **Effort:** M
- **Risk:** Medium; must not deadlock on elicitation/approval waits.

### P-34: Request-scoped confirmation channel with TTL
- **Problem:** `confirmation_rx` is one mpsc per agent; a concurrent turn or a stale/duplicate `/action-required` POST can deliver a confirmation to the wrong pending request, and a lost confirmation blocks the turn **forever** (no TTL, no "prompt expired" path) (`internal/server-flow.md` gap #2, `tool_execution.rs:171-173`).
- **Proposal:** Key confirmations by request id and add a timeout on the approval wait that emits a "prompt expired" tool result and unblocks the loop.
- **Inspired by:** novel (correctness).
- **Affected code:** `crates/biorouter/src/agents/{agent.rs:152-153,1228-1236}`, `agents/tool_execution.rs:171-229`.
- **Impact:** high — removes a permanent-hang class.
- **Effort:** M
- **Risk:** Medium; a premature timeout could deny a slow human — make TTL generous/configurable.

### P-35: Make the permission wait cancellation-aware
- **Problem:** `rx.recv().await` on the confirmation channel is not in a `select!` with the cancel token; a mid-prompt cancel only works because the client closes the socket and the stream is dropped — a programmatic cancel (`/agent/stop`) would not unblock it (`internal/server-flow.md` gap #3, `internal/loop-detection.md` #7, `tool_execution.rs:171-172`).
- **Proposal:** `select!` the approval wait against the cancel token (and the TTL from P-34) so any cancel path unblocks a parked approval.
- **Inspired by:** BioRouter's own `mcp_client` `select!` on cancel.
- **Affected code:** `crates/biorouter/src/agents/tool_execution.rs:171-172`.
- **Impact:** medium.
- **Effort:** S
- **Risk:** Low.

### P-36: A real "abort this turn" endpoint
- **Problem:** `/agent/stop` only evicts the agent from the LRU while the in-flight reply task keeps its own `Arc<Agent>`, so it does *not* cancel a running turn; there is no `session_id`-addressed cancel independent of owning the SSE socket (`internal/server-flow.md` gap #4, `agent.rs:695-710`).
- **Proposal:** Give the server a per-session `CancellationToken` registry so `/agent/stop` (or a new `/agent/cancel`) actually trips the running turn's token.
- **Inspired by:** novel.
- **Affected code:** `crates/biorouter-server/src/routes/agent.rs:695-710`, `reply.rs:249`, `state.rs`.
- **Impact:** medium — enables headless/programmatic cancel and multi-client control.
- **Effort:** M
- **Risk:** Low.

### P-37: Hard cancellation inside long tool bodies
- **Problem:** Cancellation is cooperative and boundary-only — checked between awaited items, never inside a blocking tool body; a built-in tool that ignores the token keeps running until it returns, with no forced task abort (`internal/loop-detection.md` #7, `internal/core-loop.md` #9).
- **Proposal:** Run long in-process tools on abortable tasks (or in a child process reachable by kill) so a cancel can force-stop them; audit built-in tools to poll the token.
- **Inspired by:** OpenHands (STUCK breaks), process-isolated runtimes.
- **Affected code:** `crates/biorouter/src/agents/agent.rs:836-960`, built-in tool bodies in `biorouter-mcp`.
- **Impact:** medium.
- **Effort:** M/L
- **Risk:** Medium; forced aborts can leave partial state.

### P-38: Idempotency / resume token on `/reply`
- **Problem:** With `sseMaxRetryAttempts: 1`, an SSE reconnect re-POSTs and starts a *second* turn (re-appending the user message) rather than resuming the first — no turn id or resume token (`internal/server-flow.md` gap #8).
- **Proposal:** Attach a client-generated turn/idempotency id; a re-POST with the same id resumes/attaches to the existing turn instead of duplicating it.
- **Inspired by:** standard idempotency-key pattern.
- **Affected code:** `crates/biorouter-server/src/routes/reply.rs`, `ui/desktop/src/hooks/chatStreamStore.tsx:536-544`.
- **Impact:** medium — prevents duplicate turns / double side-effects on flaky networks.
- **Effort:** M
- **Risk:** Low/medium.

## Conversation invariants and compaction

### P-39: Re-run `fix_conversation` each turn, not once per reply
- **Problem:** `fix_conversation` runs once per reply; inside the multi-turn loop the agent appends a thinking message then per-tool assistant/user messages with fresh UUIDs, so the next provider call can receive two consecutive assistant messages that were never re-normalized — correctness then depends on each provider's `create_request` grouping, an implicit contract (`internal/core-loop.md` #2, `agent.rs:1934-1957`, `mod.rs:44-63`).
- **Proposal:** Re-run the (idempotent, cheap) normalizer on the agent-visible slice at the top of each turn, or make the tool_call/tool_result pairing invariant explicit per turn.
- **Inspired by:** novel (invariant hardening).
- **Affected code:** `crates/biorouter/src/conversation/mod.rs:164-221`, `agents/agent.rs` loop.
- **Impact:** medium — removes a latent provider-dependent correctness risk.
- **Effort:** S/M
- **Risk:** Low; normalizer is idempotent and tested.

### P-40: Fix silent length-truncation on Anthropic streaming
- **Problem:** The native Anthropic streaming format never populates `ProviderUsage.finish_reason`, so the length-truncation auto-continue is dead code for the default provider — a response cut off at the output limit ends the turn silently mid-sentence (`internal/core-loop.md` #1 — "the single most surprising correctness gap", `formats/anthropic.rs:637-683`).
- **Proposal:** Propagate `stop_reason == "max_tokens"` into `finish_reason` in the Anthropic streaming format (the OpenAI-compat format already does), so `agent.rs:2053` auto-continues.
- **Inspired by:** BioRouter's own OpenAI format path.
- **Affected code:** `crates/biorouter/src/providers/formats/anthropic.rs:637-683`, `providers/base.rs:303`.
- **Impact:** high — fixes silent mid-sentence truncation on the primary provider.
- **Effort:** S
- **Risk:** Low.

### P-41: Keep a recent-turn verbatim window at compaction
- **Problem:** Compaction is summarize-everything: the *entire* agent-visible history — including the most recent tool outputs, diffs, file contents, and errors — is collapsed into lossy prose; only one plain-text user message survives verbatim (`internal/compaction.md` #1 — "the single biggest fidelity regression vs SOTA").
- **Proposal:** Keep the last N turns verbatim (or a token-budgeted recent window) and summarize only the older prefix, so the freshest, most load-bearing context is not lossy.
- **Inspired by:** Claude Code / modern coding agents (keep-recent-verbatim), OpenHands condenser (`keep_first`).
- **Affected code:** `crates/biorouter/src/context_mgmt/mod.rs:50-164,286-349`.
- **Impact:** high — major fidelity improvement across the compaction boundary.
- **Effort:** M
- **Risk:** Medium; must keep tool_call/result pairs intact within the kept window.

### P-42: Head/tail-truncate an individual over-window message
- **Problem:** A single message that alone exceeds the window (e.g. a 400k-token tool result or user paste) is a hard dead end — after removing all *whole* tool responses, `do_compact` errors and the loop tells the user to start a new session; there is no per-message truncation or pre-emptive clamp on tool-result size (`internal/compaction.md` #3, `internal/core-loop.md` #4/#6, `mod.rs:336-338`, `agent.rs:1967-1975`).
- **Proposal:** Add head/tail middle-out truncation ("…N tokens elided…") for a single oversized message, and/or a token-aware clamp on tool results before they enter history (complementing the 200k-char temp-file handler).
- **Inspired by:** modern agents (bounded preview + handle).
- **Affected code:** `crates/biorouter/src/context_mgmt/mod.rs:236-284`, `agents/large_response_handler.rs`.
- **Impact:** high — removes a "start over" cliff.
- **Effort:** M
- **Risk:** Medium; truncation can drop the important part — prefer head+tail.

### P-43: Concurrency guard on the check→compact→persist sequence
- **Problem:** `total_tokens` is read at turn start and written at turn end with no session-level lock; two turns racing on one session (e.g. a scheduled firing + a user turn) could double-compact or lose a compaction (`internal/compaction.md` #12). This compounds with the missing single-turn lock (P-33).
- **Proposal:** Serialize compaction under the per-session turn lock (P-33) or a dedicated session mutex.
- **Inspired by:** novel (concurrency correctness).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:1432-1510`, `session/session_manager.rs` replace path.
- **Impact:** medium.
- **Effort:** S/M (largely subsumed by P-33)
- **Risk:** Low.

### P-44: Validate and retry the compaction summary; don't summarize with the weakest model
- **Problem:** Compaction is one `complete_fast` call with no length/empty/format validation and no retry on junk output; the *fast* (cheapest/weakest, possibly smaller-context) model writes the memory the strong model then relies on, and a second overflow degrades to summary-of-summary before the 2-attempt cap bails (`internal/compaction.md` #2/#9/#11).
- **Proposal:** Validate the summary (non-empty, has the mandated sections) and retry once on failure; optionally use the main model (or a mid-tier) for compaction to protect fidelity.
- **Inspired by:** OpenHands condenser (minimum-progress guard), Codex dual-strategy compaction.
- **Affected code:** `crates/biorouter/src/context_mgmt/mod.rs:286-349,317-318`.
- **Impact:** medium.
- **Effort:** M
- **Risk:** Low/medium; using a stronger model raises compaction cost.

### P-45: Progressive context-overflow fallback instead of a hard 2-attempt cliff
- **Problem:** Context-overflow recovery is a hard 2-attempt cliff — after two failed compactions it simply stops, with no progressive fallback (drop oldest turns, summarize more aggressively, or switch to a larger-context model) (`internal/core-loop.md` #6, `agent.rs:1967`).
- **Proposal:** On repeated overflow, escalate: drop oldest agent-visible turns, then offer/auto-switch to a larger-context model, before giving up.
- **Inspired by:** agents that transparently swap to a larger window.
- **Affected code:** `crates/biorouter/src/agents/agent.rs:1964-2019`, `context_mgmt/mod.rs`.
- **Impact:** medium.
- **Effort:** M
- **Risk:** Medium; model switching changes cost/behavior mid-session.

## Tool execution safety and enforced verification

### P-46: Bound tool parallelism and add write-side ordering
- **Problem:** `select_all` over all approved tool futures has no concurrency cap and no cross-tool isolation, so an assistant message with many write-side calls (e.g. concurrent edits to the same file) runs them all at once with no ordering guarantees (`internal/core-loop.md` #8, `agent.rs:1792`).
- **Proposal:** Cap concurrent tool dispatch and serialize write-side tools that target overlapping paths.
- **Inspired by:** novel (safety of concurrent edits).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:708-745,1792-1843`.
- **Impact:** medium — prevents corrupt concurrent writes.
- **Effort:** M
- **Risk:** Medium; serialization can slow legitimate parallel work — scope to write-side/overlapping only.

### P-47: Post-edit diagnostics feedback loop (enforced verification)
- **Problem:** `text_editor` writes never trigger a build/lint/typecheck; the agent only learns of breakage if it *chooses* to run tests, and "done" in interactive chat is whatever the model decides — the only hard gate (`execute_success_checks`) is workflow-only (`internal/verification.md` #1/#3, `internal/state-awareness.md` #7).
- **Proposal:** Add a PostToolUse-style "run diagnostics after edit" hook (LSP/typecheck/project lints) that feeds results back automatically; optionally an enforced completion gate for interactive coding sessions.
- **Inspired by:** Claude Code / Cursor edit→diagnostics loop, OpenHands critic refine.
- **Affected code:** `crates/biorouter-mcp/src/developer/` (edit path), `crates/biorouter/src/agents/agent.rs` Stop handling, builds on P-26.
- **Impact:** high — turns "run tests" from a prompt suggestion into an enforced signal.
- **Effort:** L
- **Risk:** Medium; diagnostics must be fast and not derail unrelated tasks.

### P-48: Wire the dormant `structured_output` validate/re-prompt loop
- **Problem:** `structured_output.rs` has parse/validate/`reprompt_message` primitives with tests but **zero call sites**, so any BRSDK app relying on the `output_type` contract currently gets no enforcement (`internal/verification.md` #2, `agents/mod.rs:23`).
- **Proposal:** Wire `structured_output` into the agent loop for BRSDK `output_type` (validate the terminal message, re-prompt up to N times), mirroring the working `final_output_tool` path.
- **Inspired by:** BioRouter's own `final_output_tool` design.
- **Affected code:** `crates/biorouter/src/agents/structured_output.rs`, `agents/agent.rs` terminal-message handling.
- **Impact:** medium — a written safety net that does nothing today.
- **Effort:** M
- **Risk:** Low; primitives are tested.

## Observability and institutional policy

### P-49: Runtime observability / trace of loop-safety events
- **Problem:** `observability::{ObsEvent,TraceBuilder,TraceProcessor}` has no emit sites and `tracing/mod.rs` is a stub, so there is no runtime trace of tool-failure rates, retry counts, repetition triggers, or repair-loop firings — an operator cannot audit whether the safety mechanisms are working (`internal/verification.md` #8).
- **Proposal:** Emit the (already redaction-safe) spans at loop-safety decision points (inspector denials, retries, compaction, stop-hook blocks, cancellations) so the robustness features are observable.
- **Inspired by:** novel (operability).
- **Affected code:** `crates/biorouter/src/observability/mod.rs`, emit sites in `agents/agent.rs`, `tool_monitor.rs`, `hooks/`.
- **Impact:** medium — you cannot improve loop safety you cannot measure.
- **Effort:** M
- **Risk:** Low; ensure spans never carry args/text (the model already forbids it).

### P-50: Managed/enterprise policy tier for guardrails and hooks
- **Problem:** Both permissions and hooks have only 2 config tiers (global + opt-in project), with no non-overridable admin layer — a lab/UCSF deployment cannot enforce "no writes outside the data dir / always ask on `rm`" governance (`internal/hooks.md` #12, `compare/safety.md` "Managed/enterprise policy tier: no").
- **Proposal:** Add an admin/managed tier (ownership-verified, outside the binary) that wins over user/project config for both the command policy engine (P-21) and hooks.
- **Inspired by:** Gemini CLI (Default < Extension < User < Admin, admin wins), Claude Code managed settings.
- **Affected code:** `crates/biorouter/src/hooks/config.rs:111-143`, `security/`, `permission/`.
- **Impact:** high for institutional deployment, low for solo users.
- **Effort:** L
- **Risk:** Medium; policy resolution + ownership verification must be tamper-resistant.

## Highest-impact proposals in this lens

The proposals above are ordered by theme, not priority. These are the ones this lens rated
`Impact: high`, grouped by theme and listed in file order.

| Theme | Proposals rated high impact |
|---|---|
| Loop and stuck detection | **P-4** semantic / oscillation loop detection · **P-5** repeated-failing-result detector · **P-6** bring `/goal` stall detection to ordinary chat · **P-8** wall-clock / token / dollar budget per reply |
| Error streaks | **P-10** mistake-streak / recoverable-failure handling |
| Checkpoints and undo | **P-12** shadow-git checkpoint and task-level undo |
| Permissions and command safety | **P-19** read-only auto-approve and risk grading · **P-20** always-on catastrophic-command denylist · **P-21** auditable policy engine replacing the regex scanner · **P-22** scan tool *output* for injection and PII · **P-23** central secret-redaction boundary |
| Hooks | **P-26** let PostToolUse hooks block · **P-27** PreToolUse `updated_input` rewrite |
| Sandboxing | **P-32** OS-level sandbox for tool execution |
| Server flow | **P-33** server-enforced single-turn lock · **P-34** request-scoped confirmation channel with TTL |
| Conversation invariants and compaction | **P-40** fix silent Anthropic length-truncation · **P-41** recent-turn verbatim window at compaction · **P-42** head/tail-truncate an over-window message |
| Verification | **P-47** post-edit diagnostics feedback loop |
| Institutional policy | **P-50** managed/enterprise policy tier — rated high for institutional deployment, low for solo users |

## Related documentation

- [Master improvement proposals](../improvement-proposals.md) — the merged BR-1 … BR-67 list that superseded this lens; start here to find what actually shipped.
- [Improvement proposals — Performance and efficiency](performance.md) — the sibling lens over the same evidence, covering caching, latency and resource sharing.
- [Improvement proposals — Usability, UX and agent ergonomics](ux.md) — the sibling lens whose user-facing entries duplicate several proposals here (see the cross-lens table above).
- [Safety and guardrails, compared](../competitive-comparison/safety-and-guardrails.md) — the comparison chapter that supplies most of the "behind #N" gap citations here.
- [Agentic loop review](../README.md) — the executive report that frames all ten internal reviews and the three lenses.
