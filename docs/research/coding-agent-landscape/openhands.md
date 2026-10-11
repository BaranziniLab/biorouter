# OpenHands (All Hands AI) — agentic feedback loop review

> **What this is.** An external review of how OpenHands, an open-source AI
> software-engineering agent, implements its agentic loop — the five-heuristic
> `StuckDetector`, the structured summarizing condenser that preserves task IDs, and
> risk-graded confirmation via a per-action `security_risk` field. One of eight tool reports
> in this folder, each covering the same ten dimensions.
> **Status:** Current. External-tool research; the cited source for the
> oscillation/action-error detection in proposal BR-30 and the risk-graded permission model
> in BR-18.
> **Audience:** developers working on BioRouter's agent loop.

`BR-NN` identifiers name proposals in the agent-loop review's improvement register; the
index lives in [the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

In late 2025 OpenHands split into a modular **Software Agent SDK** that the application
repository consumes. Two repositories are therefore in play throughout this report:

| Repository | Contains | Cited paths look like |
|---|---|---|
| `OpenHands/software-agent-sdk` | The SDK: packages `openhands-sdk`, `openhands-tools`, `openhands-workspace`, `openhands-agent-server` | `openhands-sdk/openhands/sdk/…`, `openhands-tools/openhands/tools/…` |
| `OpenHands/OpenHands` (formerly `All-Hands-AI/OpenHands`) | The application that consumes the SDK | Referenced via `docs.openhands.dev` documentation |

Every claim below is grounded in the current SDK source or official docs.

> **Terminology.** The V0 monolith called these context units **"microagents"**; V1 renamed
> them **"skills."** The mechanics carried over unchanged, but older documentation and some
> deprecated directory names still use the V0 term.

> **Note.** This report does not record the date its research was performed. All source
> paths cite branch `main` with no commit pin, so re-verify constants and thresholds before
> relying on them.

## System prompt and context injection

The agent's system message is selected by `system_prompt_filename`, which defaults to
`"system_prompt.j2"` — a back-compat sentinel that resolves to a **registry preset**
(`PromptPreset.DEFAULT`); `system_prompt_planning.j2` selects `PromptPreset.PLANNING`
([`agent/base.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/agent/base.py)).
An inline `system_prompt` string or a custom `prompt_dir` with real `.j2` files is also
supported. `system_prompt_kwargs` (e.g. `{"cli_mode": True}`) parameterize the template.

Project/repo context is injected through an `AgentContext` object whose `skills` list carries
two kinds of entries
([`agent/base.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/agent/base.py)):

- `type: "repo"` — always-loaded repository context, e.g. an **`AGENTS.md`** file, injected into the system prompt at conversation start. The docs note model-specific variants (`CLAUDE.md`, `GEMINI.md`) ([skills overview](https://docs.openhands.dev/overview/skills)).
- `type: "knowledge"` — keyword-**triggered** skills carrying a `trigger: [...]` list; loaded only when a user message matches ([skills overview](https://docs.openhands.dev/overview/skills), [repo microagents](https://docs.openhands.dev/modules/usage/prompting/microagents-repo)).

`AgentContext` also supports `system_message_suffix` and `user_message_prefix` for
lightweight, always-applied steering. Skills resolve with precedence `.agents/skills/` >
`.openhands/skills/` (deprecated) > `.openhands/microagents/` (deprecated); user-level skills
live at `~/.agents/skills/` ([skills overview](https://docs.openhands.dev/overview/skills)).

A third injection mode is **path-triggered rules**: content auto-injected mid-conversation
when the agent reads/edits/creates files matching a glob — deterministic context injection
tied to file operations rather than the prompt.

## Tool loop mechanics

The loop is an append-only **event stream**. `Agent.step()`
([`agent/agent.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/agent/agent.py))
proceeds in seven stages:

1. If there are unmatched pending actions (confirmation mode), execute them and return.
2. Check whether the last user message was blocked by a `UserPromptSubmit` hook; if so, finish.
3. `prepare_llm_messages(state.view, condenser, llm)` — which may return a `Condensation` event instead of messages, short-circuiting the step.
4. `make_llm_completion(llm, messages, tools=...)` with an optional `on_token` streaming callback.
5. Parse tool calls into `ActionEvent`s.
6. Run `PreToolUse` hooks (a hook can block, producing a rejection with `rejection_source="hook"`).
7. `_execute_actions` via a `ParallelToolExecutor.aexecute_batch`, so multiple tool calls in one LLM turn run **in parallel** (`asyncio.gather`), each producing an `ObservationEvent`.

Events are typed and LLM-convertible: `MessageEvent`, `ActionEvent` (`tool_calls`),
`ObservationEvent` (`role="tool"`), `SystemPromptEvent`. `ActionEvent`s sharing an
`llm_response_id` are grouped into one assistant message with multiple `tool_calls`.
Crucially, **`Event.source` (user/agent/environment) is separate from LLM `role`**
([arch/events](https://docs.openhands.dev/sdk/arch/events)).

Error handling is graceful and non-fatal: a malformed function call
(`FunctionCallValidationError`) is fed back as a user message; a content-policy block nudges
the model to rephrase and continues; a context-window-exceeded error triggers a
`CondensationRequest` rather than crashing
([`agent/agent.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/agent/agent.py)).
Default tools are `TerminalTool` (bash), `FileEditorTool`, `TaskTrackerTool`, `BrowserToolSet`,
plus built-in `FinishTool` and `ThinkTool`
([`preset/default.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-tools/openhands/tools/preset/default.py)).

## Compaction and memory

Context is bounded by a **condenser** abstraction. The default is `LLMSummarizingCondenser`
(a `RollingCondenser`) constructed by `default_condenser` with `max_size=80, keep_first=4`
([`llm_summarizing_condenser.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/context/condenser/llm_summarizing_condenser.py)).

It computes condensation **reasons**: `REQUEST` (explicit, hard), `TOKENS` (`max_tokens`
exceeded — hard, can fail the next request), and `EVENTS` (`len(view) > max_size` — soft).
The view is split into a **pinned head** (`keep_first` events — system prompt + initial user
messages, never condensed), a **compressible middle**, and a **recent tail** kept intact; the
target is to keep roughly `max_size // 2` tail events. The middle prefix is sent to a
summarization LLM and replaced by a single `Condensation` event carrying
`forgotten_event_ids`, `summary`, and a `summary_offset`. A `minimum_progress = 0.1` guard
raises `NoCondensationAvailableException` if fewer than 10% of events would be dropped (avoids
churning). If summarization itself overflows, `hard_context_reset` retries up to 5 times, each
shrinking per-event string length by 0.8×.

The summary is **structured**, not free-form
([`summarizing_prompt.j2`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/context/condenser/prompts/summarizing_prompt.j2)):
it tracks `USER_CONTEXT`, `TASK_TRACKING` (with an explicit instruction to **preserve exact
task IDs and statuses**), `COMPLETED`, `PENDING`, `CURRENT_STATE`, and for code tasks
`CODE_STATE`, `TESTS`, `CHANGES`, `DEPS`, `VERSION_CONTROL_STATUS`. Reported benefit is up to
**2× lower per-turn API cost** with no quality loss, and flat latency in long sessions
([condenser blog post](https://www.openhands.dev/blog/openhands-context-condensensation-for-more-efficient-ai-agents)).

Cross-session memory is the **persisted event store**: sequences are stored and replayable to
resume/debug ([arch/events](https://docs.openhands.dev/sdk/arch/events)); durable project
memory lives in repo `AGENTS.md`/skills.

## Hooks and extensibility

Hooks are a first-class extension surface modeled closely on Claude Code. `HookEventType` =
`PreToolUse`, `PostToolUse`, `UserPromptSubmit`, `SessionStart`, `SessionEnd`, `Stop`; a hook
returns `HookDecision.ALLOW` or `DENY` (`ASK` reserved)
([`hooks/types.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/hooks/types.py)).
Hook scripts receive a JSON `HookEvent` on stdin with `tool_name`, `tool_input`,
`tool_response`, `message`, `session_id`, `working_dir`, `metadata`.

What they can do:

- **PreToolUse can block a tool call** before execution.
- **UserPromptSubmit can block a user message** (the step marks the conversation FINISHED).
- **The Stop hook can veto stopping** — if it denies, the run loop injects a feedback message and flips status back to `RUNNING` to keep the agent going ([`local_conversation.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/impl/local_conversation.py)).

Beyond hooks, extensibility includes MCP (Model Context Protocol) tools, custom
tools/presets, plugins/skills, and
pluggable condensers, security analyzers, and critics via discriminated-union base classes.

## Guardrails and permissions

Risk is graded per action by a `SecurityAnalyzerBase.security_risk(action) -> SecurityRisk`
(LOW/MEDIUM/HIGH/UNKNOWN); an LLM-based analyzer is enabled by default
(`llm_security_analyzer=True`)
([`security/analyzer.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/security/analyzer.py),
[`agent/agent.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/agent/agent.py)).
Analysis errors default to **HIGH risk (fail-safe)**.

A `ConfirmationPolicy` decides whether to pause: `AlwaysConfirm`, `NeverConfirm`, or
`ConfirmRisky(threshold=HIGH, confirm_unknown=True)`
([`security/confirmation_policy.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/security/confirmation_policy.py)).
`_requires_user_confirmation` exempts a lone `FinishAction`/`ThinkAction`, marks read-only
tools UNKNOWN (skipped), and otherwise sets status `WAITING_FOR_CONFIRMATION`
([`agent/agent.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/agent/agent.py)).
Confirmation is a **two-call protocol**: the first `run()` creates but does not execute
actions and waits; the next `run()` treats presence of pending actions as implicit approval
and executes them.

Execution isolation is the **sandboxed runtime**: a client-server design where OpenHands
builds a Docker "OH runtime image," launches a container, and runs an
`ActionExecutionServer`/`ActionExecutor` inside it that executes shell/file/Python actions and
returns observations
([runtime architecture](https://docs.openhands.dev/openhands/usage/architecture/runtime)). V1
abstracts this as a **Workspace** with three backends: `LocalWorkspace` (in-process),
`DockerWorkspace` (container), and `RemoteAPIWorkspace` (HTTP), so the same agent runs locally
or against a remote sandbox.

## Loop and stuck detection

The `StuckDetector` runs every iteration before `agent.step()` and, if triggered, sets status
`STUCK` and breaks the loop
([`local_conversation.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/impl/local_conversation.py)).
It scans only the **last 20 events** of the active branch, and only those **after the last
user message**
([`stuck_detector.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/stuck_detector.py)).

Five heuristics, with configurable thresholds
([`conversation/types.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/types.py)):

1. **Repeating action–observation**: last N actions and N observations all equal (`action_observation=4`).
2. **Repeating action–error**: same action N times, all observations are `AgentErrorEvent` (`action_error=3`).
3. **Monologue**: N consecutive agent `MessageEvent`s with no user interruption (`monologue=3`); condensation events don't break the streak.
4. **Alternating pattern**: an `[A,B,A,B,...]` loop where even/odd positions each match (`alternating_pattern=4`).
5. **Context-window-error loop**: repeated condensation-failure observations (currently a stubbed `return False`, blocked on an upstream issue).

Equality (`_event_eq`) compares semantic content (source, thought, action, tool_name /
observation / error text) and **ignores IDs and metrics**, so genuinely-identical retries are
caught even with fresh call IDs.

The run loop also enforces hard bounds: `max_iteration_per_run` (default **500**) → status
`ERROR` with a "maximum iterations limit" message, and an optional `max_budget_per_run`
(dollar cost) → `MaxBudgetReached`
([`local_conversation.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/impl/local_conversation.py)).

## Long-running tasks and background processes

Shell work runs through the `TerminalTool` bash session inside the sandbox; long-running or
remote jobs are served by the Docker/RemoteAPI workspace and the `openhands-agent-server`.

**Delegation** is a `DelegateExecutor` exposing two commands
([`delegate/definition.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-tools/openhands/tools/delegate/definition.py),
[`delegate/impl.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-tools/openhands/tools/delegate/impl.py)):
`spawn` creates named sub-agents (`ids`, optional `agent_types` like `researcher`/`programmer`),
and `delegate` maps `{id: task}` and runs them. Delegated tasks execute **in parallel
`threading.Thread`s and block on `join()`** for all results; sub-agents are capped by
`max_children=5`. Sub-agents get their own `LocalConversation` (with an optional confirmation
handler) and share the default condenser sizing with the parent. Built-in sub-agent
definitions (e.g. `web-researcher`) load from a `subagents/` directory; the
`TaskToolSet`/`enable_sub_agents` flag adds delegation to the default toolset
([`preset/default.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-tools/openhands/tools/preset/default.py)).
Conversations can also be **forked** for parallel exploration.

## State tracking and checkpoints

A dedicated `TaskTrackerTool` gives the agent an explicit todo list:
`TaskItem{title, notes, status ∈ todo|in_progress|done}` with commands `view` and `plan` (the
description tells the model to always `view` before editing)
([`task_tracker/definition.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-tools/openhands/tools/task_tracker/definition.py)).
A separate **planning preset** (`PromptPreset.PLANNING` / `system_prompt_planning.j2`) plus a
`PlanningFileEditor` tool provide a plan-mode variant.

Importantly, task state is **preserved across compaction**: the summarizer's `TASK_TRACKING`
section explicitly retains task IDs and statuses. Durable state/checkpointing comes from the
persisted, replayable event store (resume/debug) and conversation forking; git integration is
provided at the workspace layer (`workspace/repo.py`) rather than a bespoke undo system.

## Self-verification

OpenHands has two verification kernels.

A **critic** (`CriticMixin`) can evaluate actions before completion, with modes `all_actions`
or `finish_and_message` (only gate the terminal `FinishAction`/final message); it drives
**iterative refinement**, counting rounds via `ITERATIVE_REFINEMENT_ITERATION_KEY` and looping
the agent back if the critic is unsatisfied
([`agent/critic_mixin.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/agent/critic_mixin.py)).

A separate **goal judge** (`judge_goal`) is a pure `objective + transcript → GoalVerdict` LLM
evaluator returning a calibrated `score` (0–1 probability the objective is "provably done"), a
`complete` boolean, and a concise `missing` string describing what remains; it backs the
`/goal` runner's continuation logic and could equally back a Stop hook or status endpoint
([`conversation/goal/judge.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/goal/judge.py)).

There is no hard-wired lint/test-after-edit loop in the default agent; verification is prompt-
and critic-driven, with the sandbox making it safe to actually run tests.

## Ideas worth stealing

1. **StuckDetector's cheap, deterministic loop-breaker.** Five pattern heuristics (identical action-obs, action-error, monologue, alternating, context-error) over the last-20 events *after the last user message*, with semantic equality that ignores IDs — configurable thresholds (4/3/3/4). BioRouter's agent loop could add an analogous pre-step check to catch thrash before burning tokens, escalating to a summarize-and-retry or a user prompt. ([`stuck_detector.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/stuck_detector.py))

2. **Structured summarization schema that preserves task IDs.** The condenser prompt forces a fixed skeleton (USER_CONTEXT / TASK_TRACKING with exact IDs / COMPLETED / PENDING / CODE_STATE / TESTS / VERSION_CONTROL_STATUS) rather than a prose summary, so pruning history never loses the plan or open threads. BioRouter's `context_mgmt` pruning could adopt this template to keep todo/plan continuity through compaction. ([`summarizing_prompt.j2`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/context/condenser/prompts/summarizing_prompt.j2))

3. **Rolling condenser with pinned head + minimum-progress + hard-reset fallback.** `keep_first` pins the system prompt/goal, soft vs. hard requirements distinguish "housekeeping" from "must-shrink-or-fail-the-request," a 10% minimum-progress guard avoids churn, and `hard_context_reset` degrades gracefully by shrinking event strings. This is a robust, well-factored design worth mirroring in BioRouter's token-budget manager. ([`llm_summarizing_condenser.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/context/condenser/llm_summarizing_condenser.py))

4. **Risk-graded confirmation with a per-tool `security_risk` field + read-only exemption.** The LLM emits a risk level per call, a pluggable `ConfirmationPolicy` (`ConfirmRisky(threshold=HIGH)`) decides when to pause, read-only tools auto-pass, and analysis failures fail-safe to HIGH. This is a cleaner permission model than a static allowlist and maps well onto BioRouter's permission modes / `.biorouterignore`. ([`confirmation_policy.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/security/confirmation_policy.py))

5. **Stop-hook that can refuse to let the agent stop.** On FINISHED, a Stop hook may deny stopping and inject a continuation message, turning "done" into a checkpoint the environment can override (e.g. "tests still failing, keep going"). Combined with the goal-judge's `missing` string, this is a lightweight done-ness gate BioRouter could bolt onto its finish path. ([`local_conversation.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/impl/local_conversation.py), [`goal/judge.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-sdk/openhands/sdk/conversation/goal/judge.py))

6. **Append-only event stream with source≠role separation.** One typed, replayable log is memory, UI feed, persistence, and integration point; `Event.source` stays distinct from LLM `role`, and same-`llm_response_id` actions collapse into parallel tool calls. This underpins forking, resume, and clean condensation — a strong architectural pattern for BioRouter's session/conversation state. ([arch/events](https://docs.openhands.dev/sdk/arch/events))

7. **Named, bounded, parallel sub-agent delegation.** `spawn`/`delegate` with human-readable IDs, typed roles, `max_children=5`, and parallel threads joined for results gives concurrency without unbounded fan-out. BioRouter's `biorouter-acp` multi-agent orchestration could adopt the same bounded-parallel + blocking-join contract. ([`delegate/impl.py`](https://github.com/OpenHands/software-agent-sdk/blob/main/openhands-tools/openhands/tools/delegate/impl.py))

## Sources

| Kind | Source |
|---|---|
| Docs — event architecture | [SDK arch/events](https://docs.openhands.dev/sdk/arch/events) |
| Docs — condenser | [context-condenser guide](https://docs.openhands.dev/sdk/guides/context-condenser) |
| Docs — skills (formerly microagents) | [skills overview](https://docs.openhands.dev/overview/skills), [repo microagents](https://docs.openhands.dev/modules/usage/prompting/microagents-repo) |
| Docs — runtime | [runtime architecture](https://docs.openhands.dev/openhands/usage/architecture/runtime) |
| Paper | [SDK paper, arXiv:2511.03690](https://arxiv.org/html/2511.03690v1) |
| Blog | [context condensation post](https://www.openhands.dev/blog/openhands-context-condensensation-for-more-efficient-ai-agents) |
| Source | files under [`OpenHands/software-agent-sdk`](https://github.com/OpenHands/software-agent-sdk) |

> **Note.** Every source path cites branch `main`, not a pinned commit. Re-verify thresholds
> and defaults before relying on them.

## Related documentation

- [Gemini CLI report](gemini-cli.md) — the other layered loop detector in this corpus, for comparison against the five `StuckDetector` heuristics.
- [Cline report](cline.md) — a simpler two-tier repeated-call detector, useful as the cheap end of the design space.
- [Claude Code report](claude-code.md) — the hook event model OpenHands' `HookEventType` follows.
- [Loop and stuck detection subsystem review](../../history/agent-loop-review/subsystem-reviews/loop-and-stuck-detection.md) — how BioRouter's own detection compared when this corpus was written.
- [Improvement proposals register](../../history/agent-loop-review/improvement-proposals.md) — the `BR-NN` index, including BR-18 and BR-30.
