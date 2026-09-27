# Cline — agentic feedback loop review

> **What this is.** An external review of how Cline — an open-source autonomous coding
> agent shipping as a VS Code extension, a CLI and an SDK — implements its agentic loop,
> with emphasis on shadow-git checkpoints, three-axis restore, the mistake tracker and the
> rules system. One of nine tool reports in this folder, each covering the same ten
> dimensions.
> **Status:** Current. External-tool research, unaffected by BioRouter's own changes; it is
> the source for the shadow-git checkpoint design that proposal BR-43 implemented. Research
> date 2026-07-12, against a project that had just refactored into a monorepo, so expect
> drift.
> **Audience:** developers working on BioRouter's agent loop.

`BR-NN` identifiers name proposals in the agent-loop review's improvement register; the
index lives in [the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

Cline recently refactored into a monorepo whose runtime lives in `sdk/packages/core`, shared
by the VS Code app, the `cline` CLI and the standalone SDK; citations point at that tree
where possible. The comparison target throughout is BioRouter's agent. All claims
are cited; where docs were thin the source was fetched via `gh api repos/cline/cline`.

## System prompt and context injection

Cline's base system prompt is a template in
[`sdk/packages/shared/src/prompt/system.ts`](https://github.com/cline/cline/blob/main/sdk/packages/shared/src/prompt/system.ts).
`DEFAULT_CLINE_SYSTEM_PROMPT` opens with "You are Cline, an AI coding agent," then injects an
`<env>` block with `{{PLATFORM_NAME}}`, `{{CURRENT_DATE}}`, `{{IDE_NAME}}`, `{{CWD}}`.

The bulk of the prompt is behavioral rules: adhere to existing code conventions, use only
libraries confirmed in the codebase, no placeholder code, always use absolute paths, and
"show your planning process before executing." Notably it carries an explicit
**parallelism directive**:

> You can call multiple tools in a single response… identify every independent read, search,
> command, or edit needed for the next step and emit all of those tool calls now… Do not
> split independent reads, searches, checks, or edits across separate turns.

The prompt supplies worked "good parallelism examples" alongside it, and closes with a
done-ness rule: a "Response without tool calls will be considered completed with final
answer," and "verify the files you have edited… at the end."

A second variant, `YOLO_CLINE_SYSTEM_PROMPT`, is the autonomous/background persona —
"works in the background… a user you cannot communicate with" — that ends only when it calls
`submit_and_exit`, and mandates running the relevant test suite before completion. Both
templates end with `{{CLINE_RULES}}` and `{{CLINE_METADATA}}` interpolation points. A
`PromptRegistry` selects prompts by model family (per DeepWiki).

**Project and user context is injected via Rules.** Cline reads `.clinerules/` (all
`.md`/`.txt`, optional numeric prefixes), plus cross-tool `.cursorrules`, `.windsurfrules`,
and **`AGENTS.md`** (project root and `~/.agents/AGENTS.md`), and a global rules directory
([cline-rules docs](https://github.com/cline/cline/blob/main/docs/customization/cline-rules.mdx)).
Workspace rules win over global on conflict. Rules render into the prompt as
`## <name>\n<body>` sections under a `# Rules` heading
([`runtime/safety/rules.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/runtime/safety/rules.ts));
they are hot-reloaded from a `UserInstructionConfigWatcher` and can be individually toggled.
There is no single hard-coded `CLAUDE.md` name — the `AGENTS.md` standard is the cross-tool
convention.

## Tool loop mechanics

The loop is a `Task`/`SessionRuntime` object driven by a `Controller`
([DeepWiki architecture](https://deepwiki.com/cline/cline)): assemble system prompt → stream
the LLM via an `ApiHandler`/`ApiStream` async generator → parse streamed text and tool-use
blocks → a `ToolExecutor` dispatches to handlers (file ops, terminal, browser, and servers
speaking MCP, the Model Context Protocol) → the
loop suspends on `Task.ask()` for human approval → tool results are appended to history for
the next turn.

Tools live in
[`sdk/packages/core/src/extensions/tools/executors/`](https://github.com/cline/cline/tree/main/sdk/packages/core/src/extensions/tools/executors)
(`bash`, `editor`, `apply-patch`, `file-read`, `search`, `web-fetch`), with output-limit
handling in `output-limits.ts` and `model-tool-routing.ts` choosing tool schemas per model.

Responses are **streamed** token-by-token and parsed incrementally into
`AssistantMessageContent`. Cline strongly favors **parallel tool calls** — the system prompt
commands batching independent reads/searches/edits into one response, as quoted above. Tool
errors flow back as tool-result content and are also counted by the mistake tracker (below);
malformed tool calls surface as an `invalid_tool_call` mistake reason.

## Compaction and memory

Cline has an explicit two-strategy context pipeline in
[`extensions/context/compaction.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/extensions/context/compaction.ts).
It monitors token usage each turn against a resolved `maxInputTokens` (min of config /
model-info / context-window-derived limits) with `DEFAULT_THRESHOLD_RATIO`/`DEFAULT_TARGET_RATIO`
knobs, and runs `prepareTurn` in `auto` or `manual` mode.

- **Agentic compaction** ([`agentic-compaction.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/extensions/context/agentic-compaction.ts)):
  finds a `cutIndex` that preserves the most recent `preserveRecentTokens`, folds everything
  before it into an LLM-generated "concise continuation note with detailed next steps," carries
  forward the *previous* summary (incremental summarization), and re-attaches a **Files section**
  (`ensureFilesSection` / `extractFileOps`) so file edits survive. This maps to the user-facing
  **Auto Compact** feature ([auto-compact docs](https://github.com/cline/cline/blob/main/docs/features/auto-compact.mdx)):
  it summarizes, "preserves all technical details, code changes and decisions," replaces history,
  and "continues exactly where it left off," reusing the prompt cache so it costs about one tool
  call. A visible summarization tool call marks the event, and **checkpoints let you roll back to
  before a summarization**.
- **Basic compaction** ([`basic-compaction.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/extensions/context/basic-compaction.ts)):
  the fallback for models without agentic summary support — a rule-based truncator that trims/
  drops message text and tool-result content to a token budget while structurally **preserving
  the first user message, the last user turn, the last assistant message, and matched
  tool_use/tool_result pairs** (so no orphaned tool calls). Docs confirm non-agentic models "fall
  back to standard rule-based context truncation."

Cross-session memory is documentation-based: the **Memory Bank** methodology
([memory-bank docs](https://github.com/cline/cline/blob/main/docs/best-practices/memory-bank.mdx))
is a `.clinerules` recipe telling Cline to maintain a hierarchy of markdown files
(`projectbrief.md`, `activeContext.md`, `progress.md`, `systemPatterns.md`, etc.) and re-read
them at session start — persistence via convention, not a built-in store. Sessions themselves
persist to disk (`session/services/file-session-service.ts`) and support versioning/snapshots.

## Hooks and extensibility

Cline has a full **hooks** subsystem
([`sdk/packages/core/src/hooks/`](https://github.com/cline/cline/tree/main/sdk/packages/core/src/hooks)).
File-based hooks are discovered from a hooks config directory
([`hook-file-config.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/hooks/hook-file-config.ts))
by filename, mapping to lifecycle events: `TaskStart→agent_start`, `TaskResume→agent_resume`,
`TaskCancel→agent_abort`, `TaskComplete→agent_end`, `TaskError→agent_error`,
`PreToolUse→tool_call`, `PostToolUse→tool_result`, `UserPromptSubmit→prompt_submit`,
`PreCompact`, `SessionShutdown→session_shutdown`.

Hook scripts can be shell/`.js`/`.ts`/`.py`/`.ps1` and run as subprocesses (`subprocess.ts`,
`subprocess-runner.ts`) with a validated `HookEventPayload`. Hooks are wired in as an
`AgentExtension` carrying `capabilities: ["hooks"]`
([`hook-extension.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/hooks/hook-extension.ts)).
The naming mirrors Claude Code's hook model (PreToolUse/PostToolUse/UserPromptSubmit/PreCompact).

Beyond hooks, extensibility includes a **plugin loader** with a sandbox
(`extensions/plugin/plugin-sandbox.ts`), **MCP** servers, and **Skills** and **Rules** as
prompt-injected instruction packs. Loop detection itself is installed as a `beforeTool` hook,
and there are dedicated **checkpoint hooks** (`hooks/checkpoint-hooks.ts`) — so the internal
loop is built out of the same hook primitives exposed to users.

## Guardrails and permissions

Human-in-the-loop approval is the core guardrail: every consequential action (write, command,
browser, MCP) suspends the loop via a `ClineAsk`/`Task.ask()` prompt with a diff view. The
desktop approval channel is a file-based IPC poll
([`runtime/tools/tool-approval.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/runtime/tools/tool-approval.ts)):
it writes a `*.request.json`, polls for a `*.decision.json` (200 ms interval, **5-minute
timeout**, default-deny on timeout).

**Auto-Approve** ([auto-approve docs](https://github.com/cline/cline/blob/main/docs/features/auto-approve.mdx))
lets users grant standing approval across five categories — read files, edit files, terminal
commands, browser, MCP — with tiered toggles ("Read all files"/"Edit all files" extend outside
the workspace only when the base toggle is on) and a configurable **max-requests** cap before
re-confirmation. Rather than static allow/deny lists, the model classifies each command as
safe (builds, read-only queries) vs risky (deletes, installs, moves); risky ones still prompt.
**YOLO mode** auto-approves everything and "disables all safety checks."

Access is further gated by a `ClineIgnoreController` (`.clineignore`) and a
`CommandPermissionController`. Command execution can run in a subprocess sandbox
(`runtime/tools/subprocess-sandbox.ts`). Checkpoints are positioned as the safety net that
makes edit-auto-approve tolerable.

## Loop and stuck detection

Two dedicated, unit-testable detectors live in `runtime/safety/`:

- **Loop detection** ([`loop-detection.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/runtime/safety/loop-detection.ts)):
  a `LoopDetectionTracker` hashes each tool call into a canonical signature (`toolCallSignature`
  sorts object keys before JSON-stringifying) and counts **consecutive identical** name+signature
  calls. Thresholds default to `softThreshold: 3` → a soft warning ("consider trying a different
  approach") and `hardThreshold: 5` → a hard escalation that **stops the run** ("Detected N
  consecutive identical calls… stopping to avoid a loop"). It's installed as a `beforeTool` hook
  returning `{skip, stop, reason}`; the counter resets when a different call is made.
- **Mistake tracking** ([`mistake-tracker.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/runtime/safety/mistake-tracker.ts)):
  a `MistakeTracker` increments a `consecutiveMistakes` counter on each `api_error`,
  `invalid_tool_call`, or `tool_execution_failed` (with `forceAtLimit` to jump straight to the
  cap). Below `maxConsecutiveMistakes` it emits a recoverable error event and continues; at the
  cap it invokes an `onLimitReached` callback that can either **continue with injected guidance**
  (appended as a "recovery notice," resetting the counter) or **stop** with a preserved-state
  message ("Stopped after N/M consecutive mistakes… Session state was preserved. Send a new prompt
  to resume"). Any successful turn resets the streak. The VS Code app and CLI each have their own
  UI-side mistake surfaces (`apps/cli/src/runtime/interactive/mistakes.ts`).

## Long-running tasks and background processes

Cline has a first-class **Scheduled Agents / cron** subsystem in the core SDK
([`sdk/packages/core/src/cron/`](https://github.com/cline/cline/tree/main/sdk/packages/core/src/cron)):
spec parser, reconciler, watcher, materializer, runner with a `resource-limiter`, SQLite
store. Schedules (cron expr + prompt + workspace + model) persist across restarts via a
background **hub-spoke** daemon in `ClineCore` and route results to connectors (Slack/email)
([scheduled-agents docs](https://github.com/cline/cline/blob/main/docs/sdk/guides/scheduled-agents.mdx));
`cline schedule create …` is the CLI entry. Terminal commands can run as background processes
and the loop keeps streaming their output.

**Subagents / agent teams**
([subagents docs](https://github.com/cline/cline/blob/main/docs/features/subagents.mdx),
[`extensions/tools/team/`](https://github.com/cline/cline/tree/main/sdk/packages/core/src/extensions/tools/team))
let the main agent delegate: a `spawn_agent` tool
([`spawn-agent-tool.ts`](https://github.com/cline/cline/blob/main/sdk/packages/core/src/extensions/tools/team/spawn-agent-tool.ts))
takes a `systemPrompt` + `task`, runs a `createDelegatedAgent` in its **own context window**
with its own `maxIterations`, forwards lifecycle hooks, and returns text + per-subagent token
usage. Docs describe subagents as **read-only and non-recursive** — they can read/search/list
and run read-only commands but "cannot edit files, use the browser, access MCP servers, or
spawn nested subagents" — used to explore in parallel and return the most relevant file paths
without bloating the parent's context. Team coordination/persistence lives in `session/team/`.

## State tracking and checkpoints

**Plan/Act modes** ([plan-and-act docs](https://github.com/cline/cline/blob/main/docs/core-workflows/plan-and-act.mdx)):
Plan is read-only ("explore and strategize without changing files… cannot modify any files or
execute commands"); Act unlocks writes/commands. Full conversation history carries across the
switch, and each mode can use a different model (strong reasoner for Plan, fast model for Act).
A `/deep-planning` slash command does a systematic codebase survey and produces an
implementation plan with clarifying questions before acting.

**Focus Chain** is Cline's todo/progress tracker
([`focus-chain-utils.ts`](https://github.com/cline/cline/blob/main/apps/vscode/src/shared/focus-chain-utils.ts),
[`file-utils.ts`](https://github.com/cline/cline/blob/main/apps/vscode/src/core/task/focus-chain/file-utils.ts)).
The agent maintains a markdown checklist (`- [ ]` / `- [x]` items) persisted to
`focus_chain_taskid_<id>.md` in the task dir; it's user-editable on disk and re-ingested.
Settings ([FocusChainSettings.ts](https://github.com/cline/cline/blob/main/apps/vscode/src/shared/FocusChainSettings.ts))
are `enabled: true` with `remindClineInterval: 6` — every 6 messages Cline is reminded to
update its checklist, which is what keeps long tasks on track across compactions.

**Checkpoints** ([checkpoints docs](https://github.com/cline/cline/blob/main/docs/core-workflows/checkpoints.mdx),
[DeepWiki checkpoints and snapshots](https://deepwiki.com/cline/cline/10.1-checkpoints-and-snapshots)):
after every tool use, Cline commits workspace state to a **shadow Git repo** in VS Code global
storage (keyed by a 13-char hash of the workspace path), leaving the user's real `.git`
untouched. `GitOperations` temporarily renames nested `.git` → `.git_disabled` to avoid
submodule conflicts; `CheckpointExclusions` skips `node_modules/`, `dist/`, media/binaries and
disables checkpoints in sensitive dirs (home/Desktop/Documents/Downloads). Each checkpoint
links to a `ClineMessage` by timestamp.

Three restore modes: **Restore Files** (revert code, keep conversation), **Restore Task Only**
(truncate messages, keep code), **Restore Files & Task** (both). Captures untracked files too.
The SDK exposes `session/checkpoint-diff.ts` and `checkpoint-restore.ts`.

## Self-verification

Verification is prompt-driven rather than a hard-coded lint/test gate. The default system
prompt tells Cline to "validate the new unit test at the end including running the code if
possible" and to "verify the files you have edited… at the end of the task."

The autonomous **YOLO** prompt is stricter: after a fix it "must run the relevant test suite…
If tests fail, analyze the failures, revise your fix, and re-run until tests pass," and may
not call `submit_and_exit` until the touched files' tests pass — an explicit test-until-green
loop.

Reflection on failures is structured through the mistake tracker's `onLimitReached`
recovery-guidance path (inject a hint and keep going) rather than silently spinning. The
subagent pattern also serves verification-style research (read many files in parallel, report
findings) before the main agent edits. Editor tools apply diffs the user sees, and checkpoints
provide the objective "revert if broken" fallback.

## Ideas worth stealing

1. **Two-tier repeated-call detector as a `beforeTool` hook.** Cline's `LoopDetectionTracker`
   canonicalizes each call to a sorted-key signature and escalates soft@3 → hard@5, stopping the
   run before a wasteful loop. It's ~120 lines, pure, and unit-testable. BioRouter's Rust agent
   loop could adopt the same signature-hash + soft-warning/hard-stop split cheaply.

2. **Recoverable mistake ceiling with injected guidance.** The `MistakeTracker` counts consecutive
   `api_error`/`invalid_tool_call`/`tool_execution_failed`, and at the cap runs a callback that can
   *continue with a recovery notice* or *stop with preserved state* — not just a hard abort. This
   "one more chance with a hint" pattern is more resilient than a plain max-iterations kill.

3. **Two-strategy compaction with a preserved Files section.** Agentic (LLM incremental summary
   that re-attaches file-edit operations and carries the prior summary forward) with a rule-based
   truncation fallback that keeps first user / last turn / last assistant and matched tool pairs.
   The explicit "files survive compaction" guarantee directly addresses the classic "agent forgot
   what it edited" failure.

4. **Shadow-Git checkpoints with three restore axes.** Per-tool-use commits into a workspace-keyed
   shadow repo (untracked files included, nested-git renamed, heavy dirs excluded) plus the
   Restore-Files / Restore-Task / Restore-Both split is a clean model for undo that separates
   "code went wrong" from "conversation went wrong." A strong fit for a research agent editing
   scientific code.

5. **Focus Chain: on-disk, user-editable todo checklist with a periodic re-inject.** A markdown
   `- [ ]` list persisted per-task and re-surfaced every N (=6) messages keeps long, multi-window
   tasks coherent through compaction — cheaper and more transparent than an opaque internal plan.

6. **File-based lifecycle hooks mirroring Claude Code events.** PreToolUse/PostToolUse/
   UserPromptSubmit/PreCompact/SessionStart-style hooks that run arbitrary scripts and can
   block/inject give power users deterministic guardrails (e.g. run `cargo fmt`/clippy on
   PostToolUse). Cline even builds its own loop-detection and checkpointing on this hook bus.

7. **Cross-tool `AGENTS.md` + multi-source rules.** Reading `AGENTS.md`, `.clinerules/`,
   `.cursorrules`, `.windsurfrules`, and a global rules dir (workspace-wins-on-conflict, hot
   reloaded, individually toggleable) maximizes portability. BioRouter already has
   `.biorouterhints`; also honoring the emerging `AGENTS.md` standard would reduce onboarding
   friction.

## Sources and confidence

Grounded in Cline's live docs and `main`-branch source, fetched 2026-07-12 via `gh api`.
Primary sources: docs.cline.bot, the [cline/cline repository](https://github.com/cline/cline)
on branch `main`, and [DeepWiki's generated architecture notes](https://deepwiki.com/cline/cline)
(secondary).

- **High confidence, source-verified.** System prompt text, loop and mistake detection, compaction, hooks, focus chain, and tool-approval claims are quoted from source files.
- **High but not source-verified.** Subagent read-only/no-recursion constraints and some checkpoint internals come from docs plus DeepWiki, which is a generated secondary source.

> **Note.** Source paths cite branch `main`, not a pinned commit. Cline ships fast; the exact
> default numbers (soft=3 / hard=5, `remindClineInterval=6`, 5-minute approval timeout) are
> current as of the research date and may have drifted.

## Related documentation

- [Claude Code report](claude-code.md) — the hook event model Cline mirrors, described at its source.
- [Gemini CLI report](gemini-cli.md) — the other shadow-git checkpointing design in this corpus, with a different restore-axis split.
- [OpenCode report](opencode.md) — a third checkpoint approach (private git object DB) that avoids a shadow repo entirely.
- [Shadow-git checkpoints design](../../agent-loop/designs/shadow-git-checkpoints.md) — BR-43, the BioRouter design this report fed into.
- [Compaction and memory comparison](../../history/agent-loop-review/competitive-comparison/compaction-and-memory.md) — the head-to-head chapter scoring Cline's two-strategy pipeline against the field.
