# Gemini CLI (Google) — agentic feedback loop review

> **What this is.** An external review of Gemini CLI
> ([`google-gemini/gemini-cli`](https://github.com/google-gemini/gemini-cli)), Google's
> open-source terminal AI coding agent, covering its layered `LoopDetectionService`, its
> declarative TOML policy engine, shadow-git checkpointing with `/rewind`, and
> 30%-verbatim-tail compaction. One of nine tool reports in this folder, each covering the
> same ten dimensions.
> **Status:** Current. External-tool research; the cited source for verbatim-window
> compaction (BR-10), three-layer loop detection (BR-29/BR-30) and shadow-git rewind
> (BR-43). A July 2026 snapshot.
> **Audience:** developers working on BioRouter's agent loop.

`BR-NN` identifiers name proposals in the agent-loop review's improvement register; the
index lives in [the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

Gemini CLI is a TypeScript/Node monorepo — `packages/core` is the agent engine,
`packages/cli` the Ink/React terminal UI. BioRouter is a Rust agent whose design drew heavily
on Goose, while Gemini CLI is an *independent* TypeScript architecture: the cleanest "how would
a from-scratch competitor solve the same loop problems" reference in this corpus. Several of its
subsystems were more mature than their counterparts in Goose and in BioRouter at the time of
this report: the layered `LoopDetectionService`, the declarative TOML policy engine, shadow-git
checkpointing and rewind, and an unusually deep hooks surface. That cross-project
judgement is developed properly in the
[competitive comparison chapters](../../history/agent-loop-review/competitive-comparison/safety-and-guardrails.md);
this report records the mechanics.

> **Note.** Sources are primary: the `google-gemini/gemini-cli` source tree and official
> `docs/`, fetched July 2026 — month granularity only. All citations point at branch `main`
> with no commit pin. At 277 lines this is one of the deepest reports in the folder; siblings
> such as [Codex CLI](codex-cli.md) and [OpenCode](opencode.md) cover the same ten dimensions
> in 80-odd lines, so a thin section there is not evidence of a thin feature.

## System prompt and context injection

The system prompt is assembled in `packages/core` from a built-in **core prompt**
(safety protocol, tool-use mechanics, workflow rules) plus dynamic sections. It is fully
overridable via `GEMINI_SYSTEM_MD`: `true` reads `./.gemini/system.md`, a path reads a
custom file, and — importantly — this is a *full replacement, not a merge*, so a custom
prompt must re-include the core rules or lose them. Custom prompts can splice built-in
content back in through placeholders like `${AvailableTools}`, `${AgentSkills}`, and
per-tool name variables.
[system-prompt.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/system-prompt.md)

**Project context — `GEMINI.md` (hierarchical).** Context files are loaded from a
three-tier hierarchy, concatenated, and **sent with every prompt**: (1) global
`~/.gemini/GEMINI.md`; (2) project files discovered by walking workspace dirs and
ancestors up to a trusted root; (3) **just-in-time** — when a tool touches a
file/directory the CLI scans that location and its parents for `GEMINI.md`, so
component-specific instructions surface only when relevant (monorepo-friendly). The
filename is configurable via `context.fileName` and accepts an **array**
(`["AGENTS.md","CONTEXT.md","GEMINI.md"]`). `@file.md` imports inline another file's full
content. `/memory show` prints the exact concatenated context; `/memory reload` re-scans.
[gemini-md.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/gemini-md.md)

The docs draw a deliberate "firmware vs strategy" line: `system.md` = stable safety/tool
rules, `GEMINI.md` = project persona/goals layered on top.

## Tool loop mechanics

The loop is the `Turn` class driving `GeminiClient.sendMessageStream`, which streams model
events and hands function calls to a **tool scheduler** (`packages/core/src/scheduler/
scheduler.ts`, `tool-executor.ts`). Multiple tool calls from one model turn are **batched**
and, by default, **executed in parallel** — with two exceptions the scheduler enforces: a
`update_topic` call is forced sequential ahead of edit tools in the same batch, and a tool
can opt into `wait_for_previous: true`. A key invariant: *"we only execute if ALL active
calls are in a ready state (scheduled or terminal),"* preventing unvetted and executing
calls from interleaving.
[scheduler.ts](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/scheduler/scheduler.ts)

Each call flows through a state machine: **validating** (hook `BeforeTool` → policy/security
→ confirmation) → **scheduled** → **executing** → **success / error / cancelled**. The
executor streams partial output via an `outputUpdateHandler` for tools that
`canUpdateOutput`, and **truncates oversized output to disk**, replacing it with a
summarized reference. Errors — thrown exceptions, aborts, or tool-reported failures — are
uniformly wrapped into a `functionResponse: { error: … }` and fed back to the model so it
can self-correct rather than crashing the turn.
[tool-executor.ts](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/scheduler/tool-executor.ts)

## Compaction and memory

Compression lives in
[`packages/core/src/context/chatCompressionService.ts`](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/context/chatCompressionService.ts)
and is threshold-triggered on token usage. Every constant and function named in this list is
defined in that file:

- **`DEFAULT_COMPRESSION_TOKEN_THRESHOLD = 0.5`** (`chatCompressionService.ts`; surfaced as
  `model.compressionThreshold`, default `0.5`): when history exceeds 50% of the window,
  compress.
- **`COMPRESSION_PRESERVE_THRESHOLD = 0.3`** (`chatCompressionService.ts`): keep the **last
  30%** of history verbatim, summarize the older 70%. `findCompressSplitPoint()`, in the same
  file, snaps the boundary to the most recent user message that has no function responses, so
  the split falls on a clean conversational turn.
- **Reverse token budget**: before summarizing, `truncateHistoryToBudget()`
  (`chatCompressionService.ts`) walks backward and truncates old tool responses to 30 lines
  once they exceed `COMPRESSION_FUNCTION_RESPONSE_TOKEN_BUDGET = 50_000` tokens — a targeted
  "shrink giant old tool outputs" pass distinct from full summarization.
- **Two-phase, self-correcting summary**: an LLM produces a structured `<state_snapshot>`,
  then a **verification pass** re-reads it to recover omitted technical details.
- **Inflation guard**: if the compressed history is *larger* than the original
  (`newTokenCount > originalTokenCount`), it returns
  `COMPRESSION_FAILED_INFLATED_TOKEN_COUNT` and **discards** the result, keeping the
  original — so a bad summary never makes things worse.

`model.maxSessionTurns` (default `-1` = unlimited) is a separate hard cap on kept turns.

**Cross-session memory** is a Markdown-file model, not a database: the memory tool edits the
same three-tier `GEMINI.md` hierarchy (global preferences, project instructions, per-project
private notes) directly via write/replace.
[memory.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/memory.md)

A newer experimental **Auto Memory** runs as a background task at session start on sessions
*idle ≥ 3 h with ≥ 10 user messages*, mines transcripts for durable facts/reusable skills,
and parks candidates in a project-local **review inbox** (`/memory inbox`) for human
approval before they are ever loaded.
[auto-memory.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/auto-memory.md)

## Hooks and extensibility

Gemini CLI has an unusually deep **hooks** surface — far beyond tool-level. Events:
`SessionStart`, `SessionEnd`, `BeforeAgent`, `AfterAgent`, `BeforeModel`, `AfterModel`,
`BeforeToolSelection`, `BeforeTool`, `AfterTool`, `PreCompress`, `Notification`. Hooks are
external commands that receive JSON on **stdin** (`session_id`, `cwd`, `hook_event_name`,
plus event-specific `tool_name`/`tool_input`, `llm_request`/`llm_response`, `prompt`…) and
reply with JSON on **stdout** (exit 0), or block via exit 2 (reason on stderr).
[hooks/reference.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/hooks/reference.md)

What they can do is the interesting part:

- **`BeforeTool`** — deny, or rewrite args (`hookSpecificOutput.tool_input`).
- **`AfterTool`** — deny, or inject `additionalContext`.
- **`BeforeModel` / `AfterModel`** — modify the outgoing `llm_request` or incoming
  `llm_response` (PII redaction, synthetic responses).
- **`BeforeToolSelection`** — reshape the available `toolConfig` (mode enforcement).
- **`AfterAgent`** — can **force a retry** by returning `decision: "deny"` — a hook-driven
  self-verification lever.
- **`PreCompress`** — snapshot state before history compression.

Config is `{ matcher, sequential, hooks: [{ type:"command", command, timeout }] }`. The
broader extensibility layer is **extensions**: a `gemini-extension.json` bundle can ship
MCP (Model Context Protocol) servers over stdio, Server-Sent Events (SSE) or HTTP,
custom slash commands, prompts, themes, **hooks**,
**sub-agents**, and agent skills as one GitHub-installable unit
(`gemini extensions install <url>`).
[extensions/index.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/extensions/index.md)

## Guardrails and permissions

Two cooperating layers: coarse **approval modes** and a fine-grained **policy engine**.

Approval modes (`general.defaultApprovalMode`): `default` (prompt on each tool), `auto_edit`
(auto-approve edit tools), `plan` (read-only), and `yolo` (auto-approve everything, CLI-flag
only — cannot be set in settings). `Shift+Tab` cycles modes live.
[configuration.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md)

The **policy engine** (`docs/reference/policy-engine.md`) is declarative TOML. Each rule has
**conditions** (tool name with wildcards `mcp_*` / `mcp_*_toolName`, an args **regex**, an
`interactive` flag, and a target `approvalMode`), a **decision** (`allow` / `deny` /
`ask_user`), and a **priority** (0–999). Rules resolve by tier:
Default(1) < Extension(2) < Workspace(3, disabled) < User(4) < **Admin(5)**, with effective
priority `tier_base + toml_priority/1000` so **admin policies always win**. Files live in
per-tier dirs (`~/.gemini/policies/*.toml`; admin under `/etc/gemini-cli/policies` etc., with
ownership verification against privilege escalation). Convenience fields: `commandPrefix` /
`commandRegex` for shell, `mcpName` for MCP servers. Persistent user approvals mint a rule
scoped to the current mode *and all more permissive ones*.
[policy-engine.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/policy-engine.md)

Shell also honors `tools.core` allowlist (prefix match, e.g. `run_shell_command(git)`) and
`tools.exclude` (**checked first**, so blocklist beats allowlist).
[shell.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/shell.md)

**Sandboxing** is real OS isolation, not just gating: macOS Seatbelt profiles
(`permissive-open` default, plus `restrictive-*` / `strict-*`, each `-open` or `-proxied`
for network) and container isolation via Docker/Podman (working dir bind-mounted at the same
absolute path). Enabled with `-s` / `GEMINI_SANDBOX=docker|podman|sandbox-exec|…`;
`tools.sandboxNetworkAccess` defaults `false`.
[sandbox.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/sandbox.md)

## Loop and stuck detection

The standout subsystem is
[`packages/core/src/services/loopDetectionService.ts`](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/services/loopDetectionService.ts),
a **three-layer** real-time detector fed every streaming event and checked before each turn.
All constants below are defined in that file:

1. **Identical tool-call loop** — SHA-256 hash of `"${name}:${args}"`;
   **`TOOL_CALL_LOOP_THRESHOLD = 5`** (`loopDetectionService.ts`) consecutive identical calls
   → `CONSECUTIVE_IDENTICAL_TOOL_CALLS`. A unique call resets the counter.
2. **Content "chanting" loop** — sliding-window hashing of
   **`CONTENT_CHUNK_SIZE = 50`**-char chunks; **`CONTENT_LOOP_THRESHOLD = 10`** occurrences
   concentrated within an average distance ≤ 250 chars → `CONTENT_CHANTING_LOOP`
   (`MAX_HISTORY_LENGTH = 5000`), all in `loopDetectionService.ts`. A **list heuristic** (if
   more than half the inter-occurrence intervals differ, treat as a legitimate list) and
   code-block resets suppress false positives.
3. **LLM-based loop check** — after **`LLM_CHECK_AFTER_TURNS = 30`**, an LLM inspects the
   last **`LLM_LOOP_CHECK_HISTORY_COUNT = 20`** turns; a loop is declared only at
   **`LLM_CONFIDENCE_THRESHOLD = 0.9`**, and the check interval self-adjusts between 5–15
   turns (default 10) based on confidence.

On detection the loop yields a `LoopDetected` event and halts; a single early hit attempts
recovery. The feature is user-disable-able per session (a known UX pain point — issues
[#8237](https://github.com/google-gemini/gemini-cli/issues/8237),
[#8928](https://github.com/google-gemini/gemini-cli/issues/8928) — legitimate repetitive
edit/test cycles can trip it). Subagents add `max_turns`/`timeout_mins`/`maxActionsPerTask`
as hard bounds.

## Long-running tasks and background processes

- **Background shells**: `run_shell_command` takes `is_background: true`, returns
  immediately, and surfaces `Background PIDs` for tracking; `enableInteractiveShell` allows
  TUI/editor programs and `inactivityTimeout` kills silent processes.
  [shell.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/shell.md)
- **Subagents** (`docs/core/subagents.md`): defined as `.gemini/agents/*.md` (project) or
  `~/.gemini/agents/*.md` (user) with YAML frontmatter (name, description, tools, model,
  temperature, limits) + a Markdown-body system prompt. Each runs in a **separate context
  loop** (keeps the main history clean), with tool access via wildcards or inherited, and
  **recursion protection** (subagents cannot spawn subagents). Limits: `max_turns` (default
  **30**), `timeout_mins` (default **10**), `maxActionsPerTask`. Delegation is automatic
  (main agent picks by description) or forced via `@agent_name`.
  [subagents.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/core/subagents.md)
- No built-in cron scheduler in core; unattended runs use **headless mode** (`gemini -p`)
  driven by external schedulers, plus ACP (Agent Communication Protocol) and remote-agent
  modes.

## State tracking and checkpoints

- **Todos** (`write_todos`): the agent authors an explicit task list (states
  pending/in_progress/completed/cancelled/blocked, **only one `in_progress`**), rendered as a
  live indicator above the prompt with `Ctrl+T` to expand. It is agent-invoked (a deliberate
  planning step), session-scoped.
  [todos.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/todos.md)
- **Plan mode**: a read-only approval mode where writes are restricted to `.md` files in the
  plans dir; the agent researches, proposes a plan, and on approval **auto-exits to execute**.
  [plan-mode.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/plan-mode.md)
- **Checkpointing** (opt-in, `general.checkpointing.enabled`): before any file-modifying tool
  runs, it commits a snapshot to a **shadow git repo in the home dir** (not the project's
  repo), plus saves conversation history + the pending tool call as JSON under
  `~/.gemini/tmp/<project_hash>/checkpoints`. `/restore` reverts files, restores the
  conversation, and **re-proposes** the original tool call.
  [checkpointing.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/checkpointing.md)
- **Rewind** (`/rewind` or `Esc`,`Esc`): step back to a prior interaction and choose to revert
  **both** chat + files, **chat only**, or **files only**. It works *across compression
  points* by reconstructing from stored session data, but only undoes AI built-in-tool edits
  (not manual edits or shell side effects).
  [rewind.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/rewind.md)

## Self-verification

There is **no hard-coded lint/test-after-edit loop** in core; done-ness is "model stops
emitting tool calls," bounded by loop detection, `maxSessionTurns`, and subagent limits.
Verification is instead *composed* from other primitives: the **`AfterAgent` hook can force a
retry** (`decision:"deny"`) when a validation command fails, `AfterModel`/`AfterTool` hooks can
inspect/redact/append context, plan mode + `write_todos` structure the work, and skills or
`GEMINI.md` can instruct "run the tests / lint after editing." So Gemini CLI treats
self-verification as a hook/skill responsibility rather than a built-in agent behavior.
[hooks/reference.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/hooks/reference.md)

## Ideas worth stealing

1. **Layered `LoopDetectionService`.** The three-tier detector — SHA-256 identical-tool-call
   count (threshold 5), streaming content-chanting via 50-char chunk hashing (threshold 10
   within a tight window), and a periodic LLM sanity check after turn 30 at 0.9 confidence —
   is far more robust than Goose's single `RepetitionInspector`. BioRouter should port at
   least layers 1–2 (cheap, deterministic, run in the streaming path) with the list/code-block
   false-positive guards, since token-burning oscillation is the #1 autonomy failure.

2. **Declarative TOML policy engine with admin override.** Rules of
   `{tool-name-glob, args-regex, approvalMode, interactive} → allow|deny|ask_user` resolved by
   tier (User < Admin, admin always wins, ownership-verified) is exactly the governance layer a
   UCSF/lab deployment needs — e.g. hard-deny writes outside a data dir, or force `ask_user` on
   any `run_shell_command(rm …)` regardless of mode. It is more expressive and auditable than
   Goose's mode enum + `permission.yaml`, and lives *outside* the binary as config.

3. **Shadow-git checkpointing + `/rewind`.** Snapshot files to a **home-dir shadow repo**
   (never touching the project's git) before each edit, persist conversation + tool call, and
   offer granular revert (both / chat-only / files-only). The Goose review explicitly flags the
   *absence* of native checkpoint/undo as BioRouter's biggest safety gap versus current agents;
   Gemini CLI's design is a ready blueprint to close it.

4. **Compression inflation guard + two-phase state snapshot.** Keep the last 30% verbatim,
   summarize the rest into a structured `<state_snapshot>` with a second self-correction pass,
   and — critically — **discard the summary if it grew the token count** (`FAILED_INFLATED`).
   The inflation check and the reverse-token-budget pass that truncates only *old, huge* tool
   responses (>50k tokens → 30 lines) directly address the "one giant bioinformatics/SQL result
   poisons the window" failure without lossy whole-history summarization.

5. **Deep hook surface — especially `BeforeModel`/`AfterModel`/`AfterAgent`/`PreCompress`.**
   Goose's hooks are tool-lifecycle only; Gemini CLI hooks also intercept the *model request/
   response* and *compression*. `AfterAgent` returning `deny` to force a retry is a clean,
   external self-verification mechanism (run the tests; if they fail, make the agent try again)
   without hard-coding it in the loop — high value for reproducible research workflows.

6. **Native `write_todos` with a live pinned indicator.** A first-class, agent-authored todo
   tool (one `in_progress`, five states, rendered above the prompt) gives users legible progress
   on long multi-step tasks. Neither Goose nor BioRouter had an equivalent at the time of this
   report; it is a small addition with a large payoff for users, especially paired with plan mode.

## Sources

Primary sources only: the
[google-gemini/gemini-cli](https://github.com/google-gemini/gemini-cli) source tree and its
official `docs/` directory, fetched July 2026. Constants are quoted from source where cited,
with the defining file named at each mention.

| Topic | Source |
|---|---|
| System prompt | [system-prompt.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/system-prompt.md) |
| Context files | [gemini-md.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/gemini-md.md) |
| Tool scheduling | [scheduler.ts](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/scheduler/scheduler.ts), [tool-executor.ts](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/scheduler/tool-executor.ts) |
| Compaction | [chatCompressionService.ts](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/context/chatCompressionService.ts) |
| Memory | [memory.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/memory.md), [auto-memory.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/auto-memory.md) |
| Hooks | [hooks/reference.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/hooks/reference.md) |
| Extensions | [extensions/index.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/extensions/index.md) |
| Permissions and policy | [configuration.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/configuration.md), [policy-engine.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/reference/policy-engine.md), [shell.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/shell.md) |
| Sandboxing | [sandbox.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/sandbox.md) |
| Loop detection | [loopDetectionService.ts](https://github.com/google-gemini/gemini-cli/blob/main/packages/core/src/services/loopDetectionService.ts), issues [#8237](https://github.com/google-gemini/gemini-cli/issues/8237), [#8928](https://github.com/google-gemini/gemini-cli/issues/8928) |
| Subagents | [subagents.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/core/subagents.md) |
| State and checkpoints | [todos.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/tools/todos.md), [plan-mode.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/plan-mode.md), [checkpointing.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/checkpointing.md), [rewind.md](https://github.com/google-gemini/gemini-cli/blob/main/docs/cli/rewind.md) |

> **Note.** All source paths cite branch `main`, not a pinned commit. Re-verify constants
> before relying on line-level details.

## Related documentation

- [Goose report](goose.md) — Goose, whose `RepetitionInspector` and missing checkpoint story this report is measured against.
- [Cline report](cline.md) — the other shadow-git checkpointing design in this corpus, with a different restore-axis split.
- [Codex CLI report](codex-cli.md) — the other declarative command-policy engine, for comparison against the TOML tiers here.
- [Safety and guardrails comparison](../../history/agent-loop-review/competitive-comparison/safety-and-guardrails.md) — where the cross-project maturity judgements in this report are argued in full.
- [Shadow-git checkpoints design](../../agent-loop/designs/shadow-git-checkpoints.md) — BR-43, the BioRouter design this report fed into.
- [Improvement proposals register](../../history/agent-loop-review/improvement-proposals.md) — the `BR-NN` index, including BR-10, BR-29, BR-30 and BR-43.
