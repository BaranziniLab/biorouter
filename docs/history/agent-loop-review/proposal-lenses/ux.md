# Improvement proposals — Usability, UX and agent ergonomics

> **What this is.** The UX lens of the BioRouter agentic-loop improvement brainstorm of 2026-07-12:
> 34 proposals covering what the user sees and controls (plan modes, progress and todo visibility,
> approval fatigue, resume and undo affordances) **and** agent ergonomics (better tool descriptions,
> repo maps, verification feedback loops, done-ness signals, self-repair).
> **Status:** Historical record — this lens was merged into the master list
> ([Master improvement proposals](../improvement-proposals.md)) as part of BR-1 … BR-67, and that
> merged programme was then implemented. Flagship items from this lens shipped as **BR-43**
> (checkpoints and rewind), **BR-1** (repo map), **BR-18** (read-only auto-approve), **BR-47**
> (post-edit diagnostics) and **BR-46** (Anthropic `finish_reason`). Treat this file as the record of
> the reasoning, not as an open work queue.
> **Audience:** developers working on the desktop UI and the agent loop, and maintainers tracing why
> a BR-numbered change was made.
> **Identifier key.** `P-NN` numbers are **local to this file**. Each of the three lens files
> restarts its numbering at `P-1`, so `P-13` here is a different proposal from `P-13` in the
> performance or robustness lens; the master list disambiguates them as `ux P-13`. `BR-NN` numbers
> are the merged master-list ids, indexed in
> [Master improvement proposals](../improvement-proposals.md).

This is one of three lens files that read the same evidence base through one concern each — this one
through usability. It is an exhaustive brainstorm, running from quick wins to ambitious redesigns,
rather than a curated shortlist; the curation happened later, in the master list. Every proposal
carries the same seven fields (Problem, Proposal, Inspired by, Affected code, Impact, Effort, Risk)
and cites the report that establishes the gap. Effort is graded S (hours) / M (days) / L (weeks).

The lens deliberately spans two audiences, and the two `##` sections below keep them apart.
**P-1 … P-16** are user-facing product controls — things a person sees, clicks or is blocked by.
**P-17 … P-34** are agent ergonomics — changes the user never sees directly, which make the model
work better. A few proposals serve both, and say so where they do.

## Evidence base

The short paths cited throughout each proposal refer to the reviews below, which have since moved to
these locations.

| Cited as | Document |
|---|---|
| `internal/core-loop.md` | [Core loop and tool dispatch](../subsystem-reviews/core-loop-and-tool-dispatch.md) |
| `internal/state-awareness.md` | [State awareness and version control](../subsystem-reviews/state-awareness-and-version-control.md) |
| `internal/verification.md` | [Self-verification and done-ness](../subsystem-reviews/self-verification-and-doneness.md) |
| `internal/guardrails-permissions.md` | [Guardrails and permissions](../subsystem-reviews/guardrails-and-permissions.md) |
| `internal/loop-detection.md` | [Loop and stuck detection](../subsystem-reviews/loop-and-stuck-detection.md) |
| `internal/long-running.md` | [Long-running tasks and scheduling](../subsystem-reviews/long-running-tasks-and-scheduling.md) |
| `internal/server-flow.md` | [Server reply flow and session lifecycle](../subsystem-reviews/server-reply-flow-and-session-lifecycle.md) |
| `internal/hooks.md` | [Hooks system](../subsystem-reviews/hooks-system.md) |
| `internal/context-injection.md` | [Context injection and system prompt](../subsystem-reviews/context-injection-and-system-prompt.md) |
| `compare/context.md` | [Context and prompts, compared](../competitive-comparison/context-and-prompts.md) |
| `compare/safety.md` | [Safety and guardrails, compared](../competitive-comparison/safety-and-guardrails.md) |
| `compare/execution.md` | [Execution and verification, compared](../competitive-comparison/execution-and-verification.md) |
| `compare/memory.md` | [Compaction and memory, compared](../competitive-comparison/compaction-and-memory.md) |

> **Terms used below.** **MOIM** is BioRouter's per-action ambient-context block — a fresh
> `<info-msg>` user message carrying the current time, working directory and each platform
> extension's contribution, re-injected before every provider call; the acronym is never expanded in
> the codebase, and the companion
> [context-injection review](../subsystem-reviews/context-injection-and-system-prompt.md) treats it
> as "message of the moment". **BRSDK** is the BioRouter App SDK — the client library plus the
> server-side runner that a generated Agent Drafter app talks to. **LSP** is the Language Server
> Protocol.

## Contents

- [User-facing controls: undo, plan, todos, approvals, visibility](#user-facing-controls-undo-plan-todos-approvals-visibility) — P-1 … P-16
- [Agent ergonomics: verification loops, done-ness, tool feedback, self-repair](#agent-ergonomics-verification-loops-done-ness-tool-feedback-self-repair) — P-17 … P-34
- [Priority summary](#priority-summary)

## Proposals also raised by another lens

Where the same idea surfaced in more than one lens, the master list kept the richer writeup and
tagged the overlap. Several entries below restate a sibling lens entry almost verbatim; these are
this file's cross-lens duplicates.

| This file | Also raised as |
|---|---|
| P-1 (file checkpoints and `/rewind`) | `robustness P-12` |
| P-3 (persist goal state) | `robustness P-17` |
| P-5 (per-directory / per-prefix permission scoping) | `robustness P-24` |
| P-6 (revive read-only auto-approve) | `robustness P-19`, `performance P-39` |
| P-7 (staged loop nudge and honest repetition reason) | `robustness P-1`, `robustness P-2`, `robustness P-4` |
| P-9 (`shell_list` for background jobs) | `robustness P-16` |
| P-11, P-12 (addressable cancel, approval TTL) | `robustness P-34`, `robustness P-35`, `performance P-47` |
| P-16 (single-turn-per-session lock) | `robustness P-33`, `performance P-45` |
| P-17 (auto post-edit diagnostics) | `robustness P-47` |
| P-20 (per-model system-prompt variants) | `performance P-38` |
| P-21 (head/tail preview for oversized output) | `performance P-7`, `robustness P-42` |
| P-24 (wire the `structured_output` validation loop) | `robustness P-48` |
| P-25 (Anthropic `finish_reason`) | `robustness P-40` |
| P-27 (mistake-streak counter) | `robustness P-10` |
| P-29 (async subagent handle) | `performance P-40` |
| P-30 (refresh the clock and dedup MOIM) | `performance P-4`, `performance P-5` |
| P-32 (loop-level retry for streaming errors) | `robustness P-11`, `performance P-34` |
| P-33 (hook tool-input rewrite path) | `robustness P-26`, `robustness P-27` |

---

## User-facing controls: undo, plan, todos, approvals, visibility

### P-1: File checkpoints + `/rewind`
- **Problem:** BioRouter has **no checkpoint/undo of agent edits**. The only rollback is `text_editor`'s in-memory, per-file, per-process LIFO that dies with the developer server and misses shell/`write_file` writes (`internal/state-awareness.md` §3, gap #2; `internal/verification.md`; `compare/execution.md` names this "the single starkest deficit"). Aggressive autonomy is intolerable without a safety net.
- **Proposal:** Snapshot the worktree into a **private git object DB in the app data dir** before/after each model step (no commits, no branch moves, no touching the user's `.git`), and expose Cline-style three-axis restore (files / conversation / both) plus a `/rewind` slash command and a GUI rewind affordance on each turn.
- **Inspired by:** OpenCode (private git-object DB), Cline (shadow-git, 3 restore modes), Gemini CLI / Claude Code (shadow-repo + rewind).
- **Affected code:** new module in `crates/biorouter` (reuse `git2`, already in-tree for KB); `agents/agent.rs` turn boundary; `crates/biorouter-server/src/routes/session.rs`; GUI `ui/desktop/src/components/BaseChat.tsx` + a rewind control.
- **Impact:** high — closes the biggest safety-net gap vs current-gen agents.
- **Effort:** L
- **Risk:** snapshotting large worktrees per step can be slow/space-heavy; needs gitignore-aware excludes and size caps.

### P-2: Structured, per-item todo list (renderable + diffable)
- **Problem:** The todo tool is a **full-overwrite `String` blob** ("WARNING: completely replaces the existing content"), so there is no per-item state, no completion tracking the app can render, and accidental truncation is one bad model write away (`internal/state-awareness.md` gap #4).
- **Proposal:** Replace the blob with a structured `Vec<TodoItem { id, text, status: pending|in_progress|done }>` stored in `extension_data`, with add/update/complete operations (not replace-all). Render it as a live checklist in the GUI and CLI, and let MOIM re-inject a compact rendering.
- **Inspired by:** Claude Code / Cline TODO tracking.
- **Affected code:** `crates/biorouter-mcp/.../todo_extension.rs`; MOIM rendering in `extension_manager.rs:1509`; new GUI todo panel component.
- **Impact:** high — task visibility is the user's main progress signal.
- **Effort:** M
- **Risk:** schema migration of existing `todo.v0` blobs; model must adopt the new op-based tool.

### P-3: Persist goal state like todos
- **Problem:** `/goal` state is **in-memory only** (`GoalRegistry { goals: Mutex<HashMap<..>> }`), so a daemon restart silently drops an active goal while todos survive — a confusing inconsistency (`internal/state-awareness.md` gap #3).
- **Proposal:** Persist `GoalState` into `session.extension_data` (key `goal.v0`) exactly like `TodoState`, and reload on resume. Surface active-goal status in the GUI.
- **Inspired by:** novel (internal consistency fix).
- **Affected code:** `crates/biorouter/src/agents/goal.rs:99-131`; `extension_data.rs`.
- **Impact:** medium — removes a silent state-loss surprise.
- **Effort:** S
- **Risk:** low; goal judge config must serialize cleanly.

### P-4: A living plan artifact, not a one-shot planner
- **Problem:** Plan mode is a **one-shot prompt-rewrite** that produces a plan as the first user message of a fresh executor conversation; there is no maintained plan the agent checks off and verifies against, and no automatic "this is complex → plan" trigger (`internal/verification.md` gap #9).
- **Proposal:** Add a persistent plan artifact (reuse the structured-todo store) that the agent updates as it works, with a plan-completion checkpoint at turn end and a GUI plan view. Optionally add a heuristic that suggests plan mode for multi-step requests.
- **Inspired by:** Claude Code (maintained plan/TODO), OpenHands (goal-judge over plan).
- **Affected code:** `crates/biorouter/src/prompts/plan.md`; new plan store; `agents/agent.rs` done-ness path; GUI plan panel.
- **Impact:** high — turns planning into an inspectable, checkable artifact.
- **Effort:** L
- **Risk:** overlaps with P-2/P-19; needs a clear single source of truth for "the plan."

### P-5: Per-directory / per-command-prefix permission scoping
- **Problem:** `AlwaysAllow` is keyed on `blake3(tool_name + exact-JSON args)`, so "always allow `shell`" is only expressible as exact-args reuse or a blanket whitelist of *all* future invocations (incl. dangerous ones). There is no "allow reads under this dir but not writes" (`internal/guardrails-permissions.md` gap #8). This drives approval fatigue.
- **Proposal:** Add scoped permission grants — per-directory prefix, per-command-prefix (`git *`), read-vs-write — stored as rules and matched last-wins. Surface the scope choice in the confirmation card ("Allow `git` in this folder").
- **Inspired by:** OpenCode (wildcard last-wins), Gemini CLI (tiered TOML), Claude Code (allow/ask/deny rules).
- **Affected code:** `crates/biorouter/src/permission/permission_store.rs`; `permission_inspector.rs`; GUI `ToolCallConfirmation.tsx`.
- **Impact:** high — the single biggest lever on approval fatigue.
- **Effort:** M
- **Risk:** rule precedence bugs could over-grant; needs careful tests.

### P-6: Revive read-only auto-approve so SmartApprove ≠ Approve
- **Problem:** `PermissionInspector`'s `readonly_tools`/`regular_tools` sets are constructed **empty with no setter**, and the LLM permission judge is **dead code** — so SmartApprove behaves identically to Approve and over-prompts on every read (`internal/guardrails-permissions.md` gaps #1/#2; `compare/safety.md`).
- **Proposal:** Populate `readonly_tools` from the extension manager's `read_only_hint` annotations so reads auto-pass; adopt OpenHands' per-action `security_risk` + `ConfirmRisky(threshold)` for the "smart" tier rather than resurrecting the dead judge.
- **Inspired by:** OpenHands (`security_risk` + `ConfirmRisky`).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:348-351`; `permission/permission_inspector.rs:135-138`; `extension_manager.rs` (annotation plumbing).
- **Impact:** high — makes the advertised "smart" mode actually reduce prompts.
- **Effort:** M
- **Risk:** mis-annotated tools could auto-approve a write; fail-closed on unknown.

### P-7: Staged loop nudge + honest repetition reason
- **Problem:** Repetition detection is **exact-consecutive-duplicate only** and, on trigger, tells the model the generic `DECLINED_RESPONSE` ("the user declined") rather than the true "exceeded max repetitions" — actively misleading (`internal/loop-detection.md` #1/#2; `compare/safety.md`). A one-char arg change or A/B/A/B oscillation bypasses it.
- **Proposal:** Soft-warn at 3 identical calls (surface the REP-001 reason to the model), hard-stop at 5; add OpenHands-style alternating-pattern and action-*error* heuristics; and stop mislabeling the block as a user decline.
- **Inspired by:** Cline/OpenCode (3-warn/5-stop), OpenHands `StuckDetector`, Gemini CLI (three-layer).
- **Affected code:** `crates/biorouter/src/tool_monitor.rs`; `tool_inspection.rs`; the `DECLINED_RESPONSE` text in `tool_execution.rs`.
- **Impact:** high — fixes a correctness bug the model can never diagnose today.
- **Effort:** M
- **Risk:** over-aggressive stops could interrupt legitimate retries; thresholds need tuning.

### P-8: "What is the agent running now" dashboard
- **Problem:** Background shell jobs, subagents, and scheduled runs are **three disjoint in-memory systems with no unified view** of what the agent is currently running (`internal/long-running.md` gap #11).
- **Proposal:** Add a unified "active work" surface (HTTP route + GUI panel) listing background jobs (job_id, cmd, status), running subagents, and in-flight scheduled runs, with a kill/cancel affordance per item.
- **Inspired by:** Claude Code (`/tasks`, TaskStop), Gemini CLI (background PIDs surfaced).
- **Affected code:** new `crates/biorouter-server/src/routes/` endpoint aggregating `background.rs`, subagent registry, `scheduler.rs`; GUI panel.
- **Impact:** medium — user can see and stop runaway/forgotten work.
- **Effort:** M
- **Risk:** registries are per-`DeveloperServer`/in-memory; aggregation needs a shared handle.

### P-9: Surface `shell_list` so the agent can enumerate its jobs
- **Problem:** `job_id`s are ephemeral in-memory ints and the `list()` helper is `#[allow(dead_code)]` — if the agent forgets a `job_id` mid-session it cannot enumerate what it started (`internal/long-running.md` gap #3).
- **Proposal:** Wire `list()` as a `shell_list` tool returning `[{job_id, cmd, status, new_output_available}]` so the model can recover a lost handle.
- **Inspired by:** Claude Code (background task listing), Codex (`wait_agent`/thread-store).
- **Affected code:** `crates/biorouter-mcp/src/developer/background.rs:251`; tool registration in `rmcp_developer.rs`.
- **Impact:** medium — removes a dead-end for long background tasks.
- **Effort:** S
- **Risk:** low.

### P-10: Wire the orphaned `/interrupt` soft-interrupt to the desktop client
- **Problem:** The soft-interrupt route (`/interrupt` → `queue_soft_interrupt`) is a genuinely nice "inject mid-turn without cancel-and-resend" feature but is **orphaned**: no `#[utoipa::path]`, absent from `openapi.json`, not in the generated TS client, never called by the GUI (`internal/server-flow.md` gap; `internal/core-loop.md` notes soft interrupts as a good property).
- **Proposal:** Annotate the route, regenerate the OpenAPI + TS client, and add a GUI affordance (e.g. "steer" input while a turn runs) that posts to it.
- **Inspired by:** Pi (queued steering messages).
- **Affected code:** `crates/biorouter-server/src/routes/reply.rs:498-505`; `just generate-openapi`; `ui/desktop/src/hooks/chatStreamStore.tsx`.
- **Impact:** medium — unlocks a differentiating UX that already exists in the core.
- **Effort:** S
- **Risk:** low; the core plumbing is already tested.

### P-11: Addressable "cancel this turn" endpoint
- **Problem:** Cancellation only works by **closing the SSE socket**; `/agent/stop` misleadingly only evicts the agent from the LRU and does not stop a running turn, and the permission wait ignores the cancel token (`internal/server-flow.md` gaps). A programmatic/second-client cancel is impossible.
- **Proposal:** Add a `session_id`-addressed cancel route that trips the turn's `CancellationToken`, and put the permission-wait `rx.recv()` in a `select!` with the cancel token so a mid-prompt cancel unblocks cleanly.
- **Inspired by:** novel (server-flow fix).
- **Affected code:** `crates/biorouter-server/src/routes/agent.rs:695-710`; `agents/tool_execution.rs:171`; token wiring in `reply.rs`.
- **Impact:** medium — reliable stop for CLI/automation and multi-window.
- **Effort:** M
- **Risk:** must not double-cancel or leave the SSE task dangling.

### P-12: Approval-prompt TTL and "prompt expired" recovery
- **Problem:** The confirmation channel is **one mpsc per agent, not request-scoped**; a lost/stale confirmation blocks the turn **forever** (no TTL, no expiry path), and a duplicate `/action-required` POST can resolve the wrong pending request (`internal/server-flow.md` gaps).
- **Proposal:** Make confirmations request-scoped (map keyed by request id) with a TTL; on expiry, surface a "prompt expired — retry" state instead of hanging, and reject mismatched ids explicitly.
- **Inspired by:** novel (server-flow fix); RunState's persisted-pause pattern is adjacent.
- **Affected code:** `crates/biorouter/src/agents/agent.rs:152-153`; `tool_execution.rs:171-229`; `routes/action_required.rs`.
- **Impact:** medium — eliminates a class of "stuck forever" hangs.
- **Effort:** M
- **Risk:** TTL too short could cancel a user who stepped away; make it generous/configurable.

### P-13: Repo map / workspace summary in context
- **Problem:** The model is told **one line** ("Working directory: …") and nothing else about project structure; there is no file tree, no symbol map, no `ls` snapshot (`internal/state-awareness.md` gap #1; `compare/context.md` calls this BioRouter's single largest gap and worst-in-class). The user perceives the agent as clumsily rediscovering structure every session.
- **Proposal:** Inject a cached, gitignore-aware workspace summary into MOIM or the system prompt. Start with a cwd file listing (Cline/Claude-Code level); graduate to an Aider-style ranked tree-sitter repo-map with a token budget.
- **Inspired by:** Aider (ranked repo-map), Cline (`environment_details`), Claude Code (dir snapshot).
- **Affected code:** new module feeding `extension_manager.rs:1490` MOIM / `prompt_manager.rs`; reuse the `analyze` tree-sitter machinery.
- **Impact:** high — biggest single competence/UX gap per two reviews.
- **Effort:** L (M for a plain file listing)
- **Risk:** cost/staleness; must respect `.biorouterignore` and cache invalidation.

### P-14: Global token / wall-clock / dollar budget per reply, with a progress meter
- **Problem:** Only a soft 100-turn cap bounds a reply; there is **no time/token/$ budget**, and 429 backoff (~2 min/call) compounds inside it, so a throttled session runs far longer than the user expects (`compare/safety.md` #7; `internal/loop-detection.md` #6).
- **Proposal:** Add a per-reply budget (tokens/$/wall-clock) that terminates gracefully with a "budget reached, here's where I am" message, and show a live budget/turn meter in the GUI.
- **Inspired by:** OpenHands (`max_budget_per_run`), Codex (token budget re-injected each turn).
- **Affected code:** `crates/biorouter/src/agents/agent.rs` loop (`DEFAULT_MAX_TURNS` neighborhood); token accounting in `session_manager.rs`; GUI stream state.
- **Impact:** high — bounds cost and sets user expectations.
- **Effort:** M
- **Risk:** hard cutoffs mid-task frustrate users; needs a graceful wrap-up, not a kill.

### P-15: Session branching UX (fork/tree), not just `diverged_from`
- **Problem:** BioRouter has only `diverged_from` and **renumbers positional message ids on every rewrite**, which is fragile for stable references; there is no first-class branching UX (`internal/state-awareness.md` gaps #10; `compare/memory.md` names Pi's session-tree as best-in-class branching).
- **Proposal:** Add stable message ids (UUIDs, not positional) and a `/fork`/`/tree` branching UX so users can explore alternatives without clobbering history.
- **Inspired by:** Pi (`/tree`, `/fork`, `/clone`), Claude Code (rewind + worktrees).
- **Affected code:** `session_manager.rs:1836` (synthetic ids), `session.rs` diverge/edit routes, GUI history view.
- **Impact:** medium — better exploration/recovery affordances.
- **Effort:** L
- **Risk:** stable-id migration touches persistence and UI anchors broadly.

### P-16: Server-side single-turn-per-session lock
- **Problem:** There is **no server-enforced single-turn guard**; a second `/reply` (second window, CLI, retry, raced click) starts a concurrent turn on the same `Arc<Agent>`, sharing `confirmation_rx`/`soft_interrupts` and interleaving output — the user sees garbled/duplicated streams (`internal/server-flow.md` "single most important gap"). Also no idempotency on SSE reconnect (a reconnect re-POSTs and starts a second turn).
- **Proposal:** Hold a per-session turn lock/queue server-side; reject or queue a second `/reply`. Add a turn id / resume token so an SSE reconnect resumes rather than restarts.
- **Inspired by:** "state-of-the-art agents hold a per-session turn lock/queue" (`internal/server-flow.md`).
- **Affected code:** `crates/biorouter-server/src/routes/reply.rs:257`; `execution/manager.rs:84-116`.
- **Impact:** high — removes a visible correctness/UX bug under concurrency.
- **Effort:** M
- **Risk:** a stuck lock could block legitimate new turns; needs lock TTL/override.

---

## Agent ergonomics: verification loops, done-ness, tool feedback, self-repair

### P-17: Auto post-edit diagnostics (LSP/`analyze`) feedback loop
- **Problem:** `text_editor` writes **never trigger diagnostics**; `analyze` (tree-sitter) is a manual tool and an `LSP` tool "is listed as available but is not part of the developer extension's edit path" (`internal/verification.md` gap #3; `internal/state-awareness.md` gap #7; `compare/execution.md`). The agent only learns of breakage if it chooses to run the build.
- **Proposal:** On a successful `text_editor` write, automatically run diagnostics on the edited file(s) and feed failures back as an agent-visible message through a single bounded reflection channel (Aider's model, `max_reflections=3`).
- **Inspired by:** Claude Code / OpenCode (auto LSP after edit), Aider (lint/test reflection).
- **Affected code:** `crates/biorouter-mcp/src/developer/` edit path; wire the existing `analyze`/`LSP` capability; a reflection counter in `agents/agent.rs`.
- **Impact:** high — a real edit→check→fix loop for the R/Python/Rust code BioRouter targets.
- **Effort:** M
- **Risk:** noisy diagnostics could derail the model; cap reflections and scope to edited files.

### P-18: Structured tool-error taxonomy
- **Problem:** Tool errors are an **unstructured `is_error` bool + text blob**; a `cargo build` failure and a success both arrive as text, with no retryable-vs-fatal distinction and no file:line propagation (`internal/verification.md` gap #4; `internal/state-awareness.md` §5).
- **Proposal:** Add a typed error envelope (`{ kind: transient|invalid_args|tool_failure|not_found, retryable, message, structured?: {file,line,...} }`) that tools can emit and the model can branch on, while keeping a human-readable fallback.
- **Inspired by:** OpenHands / Gemini CLI (typed `functionResponse:{error}` the model self-corrects on).
- **Affected code:** `crates/biorouter/src/agents/tool_execution.rs:111-116`; `conversation/tool_result_serde.rs`.
- **Impact:** medium — cleaner self-correction, fewer blind retries.
- **Effort:** M
- **Risk:** MCP tools return opaque errors; taxonomy must degrade gracefully.

### P-19: Make a done-ness gate available in interactive chat
- **Problem:** Enforced verification exists **only for workflows** (`execute_success_checks`), is single-variant (`Shell`), and on failure **discards all progress** by resetting to initial messages. In interactive chat, "done" is whatever the model decides (`internal/verification.md` gaps #1/#5/#6; `compare/execution.md`).
- **Proposal:** Make the mature `/goal` Stop-hook + a shell success-check a default-capable done-ness gate in interactive chat; add non-`Shell` check variants (file-exists, output-contains, JSON-schema); on failure, surface *what failed* and iterate on the diff instead of resetting.
- **Inspired by:** Codex (evidence-based completion audit), OpenHands (critic + goal-judge), Claude Code (Stop-hook test gate).
- **Affected code:** `crates/biorouter/src/agents/goal.rs`, `retry.rs:191-218` (`SuccessCheck` enum), `agent.rs:2087-2150`.
- **Impact:** high — stops "done with a broken build."
- **Effort:** L
- **Risk:** default gates could over-run cheap chats; keep opt-in-by-default per session type.

### P-20: Per-model system-prompt variants
- **Problem:** One fixed `system.md` ships for **43+ providers** of wildly varying capability; the only provider-conditional transform is the toolshim JSON rewrite (`compare/context.md`; `internal/context-injection.md`). Ask-vs-act, verification rigor, and tool-use norms cannot be tuned to model strength.
- **Proposal:** Replace the single prompt with a provider/model-keyed variant table with a default fallback (Codex pattern), retaining a contract test per variant.
- **Inspired by:** Codex CLI (per-model prompt files).
- **Affected code:** `crates/biorouter/src/prompts/system.md` → variant registry; `reply_parts.rs` prompt selection.
- **Impact:** medium — better behavior on weak/local models (Llama Server, Ollama).
- **Effort:** M
- **Risk:** variant sprawl/maintenance; mitigate with a shared base + overrides.

### P-21: Head/tail preview for oversized tool output
- **Problem:** A 200,000-**char** per-content-item threshold dumps the full text to `std::env::temp_dir()` **with no head/tail preview and no line-count summary**, and assumes tools can reach a path outside the session sandbox (`internal/core-loop.md` gap #4; `compare/execution.md` names this worst-in-class).
- **Proposal:** Replace the blind dump with a bounded head/tail preview + line-count summary + a handle the shell/file tools can actually reach (in the session dir); use token-aware thresholds and account for multiple sub-threshold items.
- **Inspired by:** Claude Code / Gemini CLI (cap → file + head preview), OpenCode (prune-before-summarize).
- **Affected code:** `crates/biorouter/src/agents/large_response_handler.rs:6-77`.
- **Impact:** high — the model keeps working on huge grep/SQL/bioinformatics outputs instead of being blinded.
- **Effort:** M
- **Risk:** preview size tuning; ensure the handle path is inside the sandbox.

### P-22: Move core disciplines into `system.md`
- **Problem:** Planning/todo and tool-batching guidance live **only** in the todo/code-execution extensions' `get_moim`, so a session without those extensions loses the guidance entirely (`compare/context.md` implication #6).
- **Proposal:** Base the core planning/batching/verification norms in `system.md` so they hold regardless of which extensions are enabled; keep extension MOIM for live state, not for behavioral rules.
- **Inspired by:** Codex/Claude Code (disciplines in the base prompt).
- **Affected code:** `crates/biorouter/src/prompts/system.md`; `todo_extension.rs`/code-execution `get_moim`.
- **Impact:** medium — consistent behavior across configs.
- **Effort:** S
- **Risk:** prompt bloat; keep it tight.

### P-23: Self-critique / reflection pass on ordinary answers
- **Problem:** **Nothing re-reads the agent's own answer** for correctness, contradiction, or hallucination before returning it — despite the science-accuracy mandate in `system.md` (`internal/verification.md` gap #7). Judges exist only for `/goal` and permissions.
- **Proposal:** Add an optional, cheap self-consistency/critique pass (LLM-as-judge or a "verify claims against tool evidence" step) before finalizing, gated by task type or a user toggle, that can trigger one corrective loop.
- **Inspired by:** OpenHands (CriticMixin), Gemini CLI (verification pass on compaction).
- **Affected code:** `crates/biorouter/src/agents/agent.rs` done-ness path; reuse the goal-judge primitive.
- **Impact:** medium — fewer confidently-wrong biomedical answers.
- **Effort:** M
- **Risk:** latency/cost; must be scoped and skippable.

### P-24: Wire the dead `structured_output.rs` validation loop
- **Problem:** `structured_output.rs` provides fence-stripping, parse+validate, and a `reprompt_message` for the BRSDK `output_type` contract but has **zero call sites** — any app relying on `output_type` gets no enforcement (`internal/verification.md` gap #2).
- **Proposal:** Wire the parse/validate/re-prompt loop into the BRSDK agent path so `output_type` is actually enforced, mirroring `final_output_tool`'s corrective re-prompt.
- **Inspired by:** novel (finish the intended wiring); mirrors `final_output_tool.rs`.
- **Affected code:** `crates/biorouter/src/agents/structured_output.rs`; app agent loop in `routes/apps.rs`.
- **Impact:** medium — closes a silently-inert validation contract.
- **Effort:** S
- **Risk:** low; the primitives are tested, only wiring is needed.

### P-25: Fix Anthropic `finish_reason` so length-truncation continuation works
- **Problem:** The native Anthropic streaming format **never populates `finish_reason`**, so the length-truncation auto-continue is dead code for the default provider — a response cut at the output-length limit ends **silently mid-sentence** (`internal/core-loop.md` gap #1, "single most surprising correctness gap").
- **Proposal:** Read `stop_reason` from Anthropic's `message_delta` and map `max_tokens` → `ProviderUsage.finish_reason = Some("length")` so the existing bounded auto-continue fires.
- **Inspired by:** novel (parity with the OpenAI-compat format, which already propagates it).
- **Affected code:** `crates/biorouter/src/providers/formats/anthropic.rs:637-683`.
- **Impact:** high — eliminates silent truncated answers on the primary provider.
- **Effort:** S
- **Risk:** low; well-scoped, testable.

### P-26: Per-tool timeout + "this tool is taking a while" signal
- **Problem:** Timeouts are **per-extension (300s) only** — no per-tool budget, no adaptive timeout, and one slow tool blocks the whole turn with no progress signal (`internal/core-loop.md` gap #3; `compare/execution.md`).
- **Proposal:** Add a per-tool timeout below the extension ceiling and emit a periodic "still running" progress event to the GUI/model for slow tools.
- **Inspired by:** Claude Code (Bash 2-min default/10-min ceiling), Codex (`awaiter` background poll).
- **Affected code:** `crates/biorouter/src/agents/mcp_client.rs:357-369`; tool dispatch in `agent.rs`; SSE event in `reply.rs`.
- **Impact:** medium — the user sees liveness instead of a frozen turn.
- **Effort:** M
- **Risk:** too-tight timeouts kill legitimate long tools; per-tool defaults need care.

### P-27: Mistake-streak counter with injected recovery hint
- **Problem:** There is **no counter for consecutive** `api_error`/`invalid_tool_call`/`tool_execution_failed`; outside `/goal` an agent can retry the same failing action indefinitely (`compare/safety.md` #6; `internal/state-awareness.md` gap #8).
- **Proposal:** Track a mistake streak; below the cap, emit a recoverable error and continue; at the cap, inject a one-shot recovery notice (reset counter) or stop with preserved state.
- **Inspired by:** Cline (`MistakeTracker` + `onLimitReached`), Aider (`reflected_message`, cap 3).
- **Affected code:** `crates/biorouter/src/agents/agent.rs` main loop; `tool_monitor.rs`.
- **Impact:** medium — graceful "one more chance with a hint" instead of silent thrashing.
- **Effort:** M
- **Risk:** false streaks on legitimate iterative work; count only true failures.

### P-28: Unified, auto-promoted cross-session memory
- **Problem:** Cross-session memory is **three disjoint stores** (chatrecall substring `LIKE`, opt-in Knowledge KB, conversation-ingest) with no shared index and no auto-promotion; "Soul" is opt-in per query, never auto-injected (`internal/state-awareness.md` gaps #5/#6; `compare/memory.md`).
- **Proposal:** Index chat with SQLite FTS5 (already available) instead of substring `LIKE`, and add Codex/Claude-style auto-distillation of finished sessions into a ranked, auto-injected memory file so lab know-how accumulates without the user asking.
- **Inspired by:** Codex (`~/.codex/memories/`), Claude Code (auto-load `MEMORY.md`), Gemini (review inbox).
- **Affected code:** `crates/biorouter/src/session/chat_history_search.rs:117-172` (FTS5 index); KB ingest; a memory-injection step in `prompt_manager.rs`.
- **Impact:** high — turns strong-but-disjoint stores into effective recall.
- **Effort:** L
- **Risk:** auto-injection can leak stale/irrelevant facts; needs ranking + user review.

### P-29: Async subagent handle + structured result envelope
- **Problem:** Subagents are **fully blocking** (parent tool call parks until the child finishes) and results are **lossy** — default `summary=true` returns only the last text message, yielding "No text content in last message" if the child ends on a tool call (`internal/long-running.md` gaps #4/#5; `compare/execution.md`).
- **Proposal:** Add a spawn→poll model (`task_status`) and a typed result envelope `{status, summary, error, artifacts}` so a child ending on a tool call yields a meaningful result and long subagents don't stall the parent turn.
- **Inspired by:** OpenCode (`task(background=true)` + `task_status`), Codex (`wait_agent`/`resume_agent`).
- **Affected code:** `crates/biorouter/src/agents/subagent_tool.rs`, `subagent_handler.rs:58-114`.
- **Impact:** medium — real parallel delegation without lossy summaries.
- **Effort:** L
- **Risk:** async handles complicate the loop and persistence.

### P-30: Refresh the clock and dedup MOIM
- **Problem:** `{{current_date_time}}` is set **once at agent construction** (UTC hour granularity) while MOIM uses Local minute granularity — the model sees two contradictory clocks; and MOIM is re-injected **without dedup**, so ambient blocks accumulate (`compare/context.md` gaps #2/#3, implication #5).
- **Proposal:** Recompute `current_date_time` per turn (or drop it in favor of MOIM's timestamp), and remove the prior MOIM block before inserting the new one.
- **Inspired by:** Codex (per-turn `current_time.rs`).
- **Affected code:** `crates/biorouter/src/agents/moim.rs`; system-prompt assembly in `reply_parts.rs`/`prompt_manager.rs`.
- **Impact:** medium — removes a confusing contradiction and context bloat.
- **Effort:** S
- **Risk:** per-turn clock can bust prompt caching; keep coarse granularity.

### P-31: Richer tool-confirmation card (diff/preview + risk rationale)
- **Problem:** The confirmation card carries an inspector warning string, but security findings only annotate, and there is no consistent preview of *what the tool will do* (e.g. the diff of an edit, the exact shell command) surfaced for the approval decision (`internal/server-flow.md` §1; `internal/guardrails-permissions.md` Q3 — annotate-and-escalate).
- **Proposal:** For write-side tools, render a preview (file diff for `text_editor`, the resolved command for `shell`) plus any security/risk explanation in `ToolCallConfirmation`, so users approve with context and click "always allow" less blindly.
- **Inspired by:** Cline/Claude Code (diff previews on edit approval).
- **Affected code:** `ui/desktop/src/components/ToolCallConfirmation.tsx`; the `ActionRequired` payload in `tool_execution.rs:161-169`.
- **Impact:** medium — better-informed approvals, less fatigue-driven blanket allow.
- **Effort:** M
- **Risk:** large diffs need truncation; payload size on the SSE channel.

### P-32: Loop-level retry with backoff for streaming/provider errors
- **Problem:** A mid-stream decode error or any non-context `ProviderError` **ends the turn** with a "please retry" string and pushes the decision onto the user; the streaming path is not covered by `ProviderRetry` (`internal/core-loop.md` gap #5).
- **Proposal:** Wrap the streaming path in bounded retry-with-backoff for transient failures (network blip, 5xx) before surfacing an error, preserving partial output.
- **Inspired by:** novel (parity with the non-streaming `with_retry` path).
- **Affected code:** `crates/biorouter/src/providers/anthropic.rs:273-313`; `providers/retry.rs`; loop error handling in `agent.rs:2020-2028`.
- **Impact:** medium — fewer user-visible failures on long streams.
- **Effort:** M
- **Risk:** retrying a partially-streamed turn risks duplicated content; need careful resume semantics.

### P-33: Give hooks a tool-input rewrite path (policy engine, not just veto)
- **Problem:** Hooks can only allow/deny/ask/inject; there is **no rewrite path**, and PreToolUse `additionalContext` is silently dropped; PostToolUse is observe-only although the decision is already computed (`internal/hooks.md` #1/#2; `compare/safety.md` #1/#8). So a lab cannot auto-fix a tool call (sandbox a path, redact a payload) without prompting the user.
- **Proposal:** Copy Codex's `PreToolUseOutcome` (`should_block` / `additional_contexts` / `updated_input`) so hooks can rewrite tool args, stop dropping PreToolUse context, and let PostToolUse block (e.g. "reject a write that fails lint").
- **Inspired by:** Codex (`updated_input`), Gemini CLI (`tool_input`), Pi (mutate `event.input`).
- **Affected code:** `crates/biorouter/src/hooks/outcome.rs`, `hooks/inspector.rs`, `agents/tool_execution.rs`, `agent.rs:1848-1913`.
- **Impact:** medium — turns hooks from a veto into a governance/auto-fix layer, reducing prompts.
- **Effort:** M
- **Risk:** rewritten args bypass validation; must re-validate or clearly scope trust.

### P-34: Reasoning-effort / "explore vs. answer" control
- **Problem:** There is **no reasoning-effort or thinking-budget knob** at the loop level; the explore-vs-answer tradeoff is left entirely to the model and prompt tone (`internal/verification.md` §4). Users cannot ask for "think harder" or "answer quickly."
- **Proposal:** Surface a per-turn effort control (quick / normal / deep) that maps to thinking-budget/temperature/exploration caps and, for deep mode, enables the self-critique pass (P-23) and a done-ness gate (P-19). Expose it as a GUI toggle and a slash flag.
- **Inspired by:** Claude Code / Codex (effort tiers), subagent `max_turns` (the only existing explore budget).
- **Affected code:** `crates/biorouter/src/agents/agent.rs` loop config; `SessionConfig`; provider params in `providers/base.rs`; GUI control.
- **Impact:** medium — user control over the depth/latency tradeoff.
- **Effort:** M
- **Risk:** provider support for thinking budgets varies; degrade gracefully where unsupported.

---

## Priority summary

**Highest-value, user-visible.**

| Proposal | Title |
|---|---|
| P-1 | File checkpoints + `/rewind` |
| P-13 | Repo map / workspace summary in context |
| P-6 | Revive read-only auto-approve so SmartApprove ≠ Approve |
| P-5 | Per-directory / per-command-prefix permission scoping |
| P-16 | Server-side single-turn-per-session lock |
| P-25 | Fix Anthropic `finish_reason` so length-truncation continuation works |

**Highest-value, agent ergonomics.**

| Proposal | Title |
|---|---|
| P-17 | Auto post-edit diagnostics (LSP/`analyze`) feedback loop |
| P-19 | Make a done-ness gate available in interactive chat |
| P-21 | Head/tail preview for oversized tool output |
| P-28 | Unified, auto-promoted cross-session memory |

**Quick wins**, each graded Effort S.

| Proposal | Title |
|---|---|
| P-3 | Persist goal state like todos |
| P-9 | Surface `shell_list` so the agent can enumerate its jobs |
| P-10 | Wire the orphaned `/interrupt` soft-interrupt to the desktop client |
| P-22 | Move core disciplines into `system.md` |
| P-24 | Wire the dead `structured_output.rs` validation loop |
| P-25 | Fix Anthropic `finish_reason` so length-truncation continuation works |
| P-30 | Refresh the clock and dedup MOIM |

## Related documentation

- [Master improvement proposals](../improvement-proposals.md) — the merged BR-1 … BR-67 list that superseded this lens; start here to find what actually shipped.
- [Improvement proposals — Robustness and safety](robustness.md) — the sibling lens whose entries duplicate several proposals here (see the cross-lens table above).
- [Improvement proposals — Performance and efficiency](performance.md) — the sibling lens covering caching, latency, startup and resource sharing.
- [Execution and verification, compared](../competitive-comparison/execution-and-verification.md) — the comparison chapter that names checkpoints and oversized-output handling as the starkest deficits.
- [Agentic loop review](../README.md) — the executive report that frames all ten internal reviews and the three lenses.
