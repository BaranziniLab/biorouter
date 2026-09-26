# Claude Code (Anthropic) — agentic feedback loop review

> **What this is.** An external review of Claude Code, Anthropic's terminal-native coding
> agent (CLI + IDE extensions + web/cloud), treated as the reference design this corpus
> benchmarks BioRouter against. One of nine tool reports in this folder, each covering the
> same ten dimensions.
> **Status:** Current. External-tool research; the cited source for hook-model parity
> claims, tool-output offloading (BR-6) and lower-trust project files (BR-9). Pinned to the
> Claude Code v2.1.x line and to a closed-source product, so it will date faster than the
> open-source reports here.
> **Audience:** developers working on BioRouter's agent loop.

`BR-NN` identifiers name proposals in the agent-loop review's improvement register; the
index lives in [the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

Claude Code is the reference design for the current generation of coding agents. Its loop is
proprietary (minified JS, no source repo), so this report leans on Anthropic's **official
docs** (`code.claude.com/docs`, `platform.claude.com`) as primary sources, supplemented by
credible reverse-engineering write-ups where the docs are deliberately silent — chiefly the
system prompt. Every claim is cited, and the citation itself is the provenance marker: a
`code.claude.com` or `platform.claude.com` link is official, anything else is third-party.

> **Warning.** Product version references are to the Claude Code v2.1.x line, with docs
> fetched July 2026 (the report does not record a more precise research date). Claude Code
> is closed-source and ships continuously; verify any version-specific claim against current
> documentation before relying on it.

## System prompt and context injection

> **Provenance.** The prompt-structure claims in this section come from third-party
> reverse-engineering teardowns, not Anthropic documentation. The `CLAUDE.md` hierarchy and
> context-injection claims that follow are from the official memory docs.

Claude Code assembles its system prompt from **static + dynamic** sections separated by a
literal cache boundary marker (`__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__`) so the static prefix can
be prompt-cached across turns. The prompt opens by fixing identity — *"You are an interactive
agent that helps users with software engineering tasks"* — then a **Tone & Style** section
(concise, "if you cannot help, do not explain why"), a **System Rules / injection-defense**
section, an **Executing Actions with Care** section ("check with the user before proceeding"
for hard-to-reverse actions), and a **Using Your Tools** section ("Do NOT use the Bash to run
commands when a relevant dedicated tool is provided").
[dbreunig system-prompt teardown](https://www.dbreunig.com/2026/04/04/how-claude-code-builds-a-system-prompt.html)

The persona is engineered in the prompt, not fine-tuned, and uses XML tags —
`<system-reminder>` injected at end of turns to re-assert rules, plus `<good-example>` /
`<bad-example>` few-shot blocks.
[weaxs prompt analysis](https://weaxsey.org/en/articles/2025-10-12/)

The system prompt itself is only ~4,200 tokens; the bulk of durable context is injected as a
**user message after the system prompt** — critically, **CLAUDE.md is delivered as a user
message, not as part of the system prompt**, which is why adherence is best-effort rather than
enforced.
[memory docs](https://code.claude.com/docs/en/memory) ·
[context-window docs](https://code.claude.com/docs/en/context-window)

**Project context files (CLAUDE.md hierarchy).** Loaded broadest-to-most-specific, root-down,
so the most local file is read last: **managed policy** (`/Library/Application
Support/ClaudeCode/CLAUDE.md`, `/etc/claude-code/CLAUDE.md`) → **user** (`~/.claude/CLAUDE.md`)
→ **project** (`./CLAUDE.md` or `./.claude/CLAUDE.md`) → **local** (`CLAUDE.local.md`,
gitignored). Ancestor files load in full at launch; **nested CLAUDE.md in subdirectories load
on demand** when Claude reads a file there. `@path` imports inline other files (max depth 4).
Claude Code reads **`CLAUDE.md`, not `AGENTS.md`/`GEMINI.md`** — the documented pattern is to
put `@AGENTS.md` at the top of a `CLAUDE.md` or symlink it. `.claude/rules/*.md` hold modular
rules; a `paths:` frontmatter glob makes a rule **path-scoped** (loads only when Claude touches
matching files). An **environment-info block** (cwd, platform, shell, OS, git branch/status/
recent commits) is appended at the very end of the system prompt.
[memory docs](https://code.claude.com/docs/en/memory)

Context is also injected **mid-conversation**: path-scoped rules fire when a matching file is
read; SessionStart/UserPromptSubmit/PostToolUse hooks can inject `additionalContext`; and after
compaction the project CLAUDE.md is **re-read from disk and re-injected**.

## Tool loop mechanics

Claude drives a fixed set of built-in tools whose exact names double as permission-rule and
hook-matcher identifiers: `Bash`, `Read`, `Edit`, `Write`, `Glob`, `Grep`, `Agent` (subagents),
`Task*` / `TodoWrite` (task list), `WebFetch`, `WebSearch`, `NotebookEdit`, `LSP`, `Monitor`,
`Skill`, plus scheduling/notification tools.
[tools reference](https://code.claude.com/docs/en/tools-reference)

- **Parallelism.** Claude emits multiple tool calls in one turn and they run as a **batch**;
  the `PostToolBatch` hook fires "after a full batch of parallel tool calls resolves, before the
  next model call," confirming concurrent dispatch.
  [hooks reference](https://code.claude.com/docs/en/hooks)
- **Bash.** Runs each command in a fresh process (env vars don't persist; `cd` carries over
  only inside the project dir). **2-minute default timeout, 10-minute ceiling**
  (`BASH_MAX_TIMEOUT_MS`). **Output capped at 30,000 chars** (ceiling 150,000): overflow is
  **written to a file in the session dir and Claude gets the path + a head preview**, then
  reads/greps the file for the rest — a key context-hygiene trick. `run_in_background: true`
  detaches long processes; `/tasks` lists/stops them.
- **Edit.** Exact-string replace guarded by **read-before-edit** (Claude must have Read the file
  this session and it must be unchanged on disk), an exact-match check, and a **uniqueness**
  check (or `replace_all`). `Write` to an existing file similarly requires a prior read.
- **Read.** Returns line-numbered content; large files return a first page with a `PARTIAL view`
  notice and `offset`/`limit` paging. Handles images, PDFs (`pages` ranges), and notebooks.
- **Error handling.** A failed tool returns its error to the model as the tool result so it can
  self-correct; WebFetch is "lossy by design" (a small model extracts against a prompt). MCP
  (Model Context Protocol) tool schemas can be **deferred** — only names are listed until
  `ToolSearch` loads the schema.

## Compaction and memory

Context management is a first-class, two-layer system.
[context-window docs](https://code.claude.com/docs/en/context-window) ·
[compaction (platform)](https://platform.claude.com/docs/en/build-with-claude/compaction)

- **Auto-compact** fires when usage approaches the window — reported around **~83.5%** in
  recent builds (raised from ~77–78%).
  [ClaudeLog auto-compact FAQ](https://claudelog.com/faqs/what-is-claude-code-auto-compact/)
  Manual `/compact [instructions]` compacts on demand and accepts preservation hints.
- **The compaction summary is structured.** It keeps: "your requests and intent, key technical
  concepts, files examined or modified with important code snippets, errors and how they were
  fixed, pending tasks, and current work." It **replaces the verbatim conversation — full tool
  outputs and intermediate reasoning are gone.**
  [context-window docs](https://code.claude.com/docs/en/context-window)
- **What survives.** Startup auto-loads are re-injected: system prompt and **project-root
  CLAUDE.md are re-read from disk after `/compact`**. What's *lost*: nested subdirectory
  CLAUDE.md (until re-read), path-scoped rules, exact file contents read mid-session, and any
  instruction given only in chat.
  [memory docs: instructions lost after /compact](https://code.claude.com/docs/en/memory)
- **Tool-output offloading** (a micro-compaction relative): Bash overflow → file; deferred MCP
  schemas. `PreCompact`/`PostCompact` hooks can block or observe compaction; `SessionStart` with
  matcher `compact` re-injects context afterward.

**Cross-session memory** has two tracks: **CLAUDE.md** (you write, instructions) and **Auto
memory** (Claude writes, learnings). Auto memory lives at
`~/.claude/projects/<project>/memory/` — a `MEMORY.md` index whose **first 200 lines / 25 KB
load into every session**, with topic files read on demand. It's per-repo, machine-local,
shared across worktrees, and agent-writable ("remember that…").
[memory docs: auto memory](https://code.claude.com/docs/en/memory#auto-memory)

## Hooks and extensibility

Hooks are Claude Code's deterministic control layer — external scripts (or HTTP/MCP/prompt/agent
handlers) wired to lifecycle events in `settings.json` under `hooks{}`, filtered by `matcher`
(tool name, regex, or `|`-list) and gated by exit code (0 = apply JSON, **2 = block**).
[hooks reference](https://code.claude.com/docs/en/hooks)

The event surface is now very broad (~30 events). The ones this review asked about:

- **PreToolUse** — before a tool runs; richest control: `permissionDecision` of
  **`allow` / `deny` / `ask` / `defer`**, plus `updatedInput` (rewrite tool args) and
  `additionalContext`.
- **PostToolUse** — after success; can `updatedToolOutput` (redact/replace the result),
  inject `additionalContext`, or `decision: "block"` to force a follow-up.
- **Stop / SubagentStop** — when the agent tries to end its turn; `decision: "block"` with a
  `reason` **keeps it working**, or `additionalContext` feeds non-error feedback back in.
- **PreCompact / PostCompact** — block or observe compaction (`trigger_type: manual|auto`).
- **SessionStart** — matchers `startup|resume|clear|compact`; injects `additionalContext`,
  `initialUserMessage`, `sessionTitle`, and can `reloadSkills`.
- **UserPromptSubmit** — stdout becomes context; can `decision: "block"` a prompt.
- Newer additions worth noting: **PostToolBatch** (veto the loop after a parallel batch),
  **PermissionRequest** (auto-answer approval dialogs), **InstructionsLoaded** (audit which
  CLAUDE.md/rules loaded), **TaskCreated/TaskCompleted** (veto task lifecycle), **ConfigChange**.

Broader extensibility: **MCP servers** add tools; **plugins** bundle hooks/commands/skills/
monitors; **skills** are prompt-workflows run through the `Skill` tool (loaded on demand, so
zero context cost until invoked). Hook scripts get `${CLAUDE_PROJECT_DIR}` / `${CLAUDE_PLUGIN_ROOT}`.

## Guardrails and permissions

Six permission modes, cycled with **Shift+Tab** (default → acceptEdits → plan) or set via
`--permission-mode` / `defaultMode`:
[permission modes](https://code.claude.com/docs/en/permission-modes)

- **`default` (Manual)** — reads only run without asking; every write/exec/network action prompts.
- **`acceptEdits`** — auto-approves file edits and a fixed set of filesystem Bash commands
  (`mkdir touch rm mv cp sed …`) **inside the working dir**; riskier commands still prompt.
- **`plan`** — read-only investigation; Claude writes a plan and touches nothing until approved.
- **`auto`** — no routine prompts, but **a separate classifier model vets every shell/network
  action** before it runs, blocking escalation, unrecognized infra, or hostile-content-driven
  actions (`curl|bash`, force-push, prod deploys, mass deletes, secret exfiltration, etc.).
- **`dontAsk`** — auto-denies anything not pre-approved (locked-down CI).
- **`bypassPermissions`** (`--dangerously-skip-permissions`) — skips all checks; refuses to run
  as root; `rm -rf /` still prompts as a circuit-breaker.

**Protected paths** (`.git`, `.claude`, shell rc files, `.npmrc`, `.mcp.json`, …) are **never
auto-approved in any mode except bypass**, and allow-rules can't override them. On top of modes,
`permissions.allow/ask/deny` rules (`Bash(npm run *)`, `Edit(/src/**)`, `WebFetch(domain:…)`)
apply in every mode; **deny and ask always win**. A separate **sandbox** provides OS-level
filesystem+network isolation for Bash.
[sandboxing](https://code.claude.com/docs/en/permission-modes)

The auto-mode classifier is a strong prompt-injection design: it **sees user messages, tool
calls, and CLAUDE.md but tool *results* are stripped**, so hostile file/web content can't steer
it directly, and a server-side probe flags suspicious tool results before Claude reads them.
Boundaries the user states in chat ("don't push") are treated as block signals.

## Loop and stuck detection

> **Provenance.** The bullets below are documented behaviour. The closing caveat is an
> inference, not a documented mechanism, and is marked as such.

Claude Code's public docs are thin on an explicit repetitive-call detector in the main loop, but
several external termination guarantees exist:

- **Auto-mode block fallback.** If the classifier blocks **3 actions in a row, or 20 total**,
  auto mode **pauses and reverts to prompting**; in headless `-p` mode repeated blocks **abort
  the session**. Any allowed action resets the consecutive counter. These thresholds are not
  configurable.
  [permission modes: when auto mode falls back](https://code.claude.com/docs/en/permission-modes)
- **Subagent turn caps.** A subagent's `maxTurns` frontmatter bounds how long a delegated worker
  runs before returning.
  [sub-agents](https://code.claude.com/docs/en/sub-agents)
- **Stop-hook loop guard.** A `Stop` hook that keeps blocking to force more work is itself a
  potential infinite loop; the hook contract passes the prior stop state so a hook can avoid
  re-blocking. (Anthropic doesn't publish a numeric cap the way Goose does.)

> **Caveat.** The minified client almost certainly contains an internal max-iteration /
> duplicate-call guard, but it is not documented as a stable, configurable knob, so this
> report asserts no specifics.

## Long-running tasks and background processes

Claude Code has an unusually rich set of background/async primitives:
[tools reference](https://code.claude.com/docs/en/tools-reference)

- **Background Bash** (`run_in_background: true`) for dev servers / watch builds; managed via
  `/tasks`, `TaskOutput` (deprecated in favor of `Read` on the output file), `TaskStop`.
- **Monitor tool** — runs a script (or opens a **WebSocket**) in the background and feeds each
  output line back to Claude mid-conversation: tail a log and flag errors, poll a PR/CI job,
  watch a directory. It reacts without pausing the turn.
- **Subagents** — spawned via the `Agent` tool into a **separate context window**; **run in the
  background by default** (v2.1.198), returning only a final text summary to the parent. Built-in
  `general-purpose`, `Explore`, and `Plan` agents (the latter two skip loading CLAUDE.md for a
  smaller context). The `Agent` tool is withheld from subagents by default to prevent recursion.
  [sub-agents](https://code.claude.com/docs/en/sub-agents)
- **Workflow tool** — a script that orchestrates many subagents in the background and returns one
  consolidated result. **Agent teams** run peer sessions that message each other.
- **Scheduling.** `CronCreate/List/Delete` schedule session-scoped recurring prompts; `/schedule`
  (`RemoteTrigger`) creates cloud **Routines** on a cron; `/loop` + `ScheduleWakeup` lets Claude
  self-pace a recurring loop (1 min–1 hr). `PushNotification` reaches you when a long job finishes.

## State tracking and checkpoints

- **Task list.** `TaskCreate / TaskGet / TaskList / TaskUpdate` manage a structured checklist
  (with dependencies); this **superseded `TodoWrite`** as of v2.1.142 (`TodoWrite` remains for
  its `pending` / `in_progress` / `completed` states via `CLAUDE_CODE_ENABLE_TASKS=0`). The
  `TaskCreated`/`TaskCompleted` hooks can **veto** creation/completion (exit 2 / `continue:
  false`), enforcing done-ness gates.
  [tools reference](https://code.claude.com/docs/en/tools-reference)
- **Plan mode.** `EnterPlanMode`/`ExitPlanMode` (or `/plan`, Shift+Tab): Claude researches
  read-only and presents a plan; approving it switches into auto/acceptEdits/manual and **names
  the session from the plan**. `Ctrl+G` opens the plan in an editor before proceeding.
  [permission modes: plan](https://code.claude.com/docs/en/permission-modes)
- **Git worktrees.** `EnterWorktree` / `isolation: worktree` run edits in an isolated worktree
  under `.claude/worktrees/`, so a subagent's changes are physically separated from your tree.
- **Checkpoints / rewind.** Claude Code maintains automatic edit checkpoints and a **rewind**
  control that restores files + conversation to an earlier state — a documented product feature.
  Its dedicated documentation page was not fetched for this report, so the exact mechanics
  should be treated as approximate. This is the one safety net that Goose lacks entirely,
  and that BioRouter lacked when this report was written.

## Self-verification

Claude Code bakes verification into the loop at several points:

- **LSP after-edit diagnostics.** The `LSP` (Language Server Protocol) tool **automatically
  reports type errors and warnings after each file edit**, so Claude fixes issues without a
  separate build step — a
  built-in edit→check→fix micro-loop.
  [tools reference: LSP tool behavior](https://code.claude.com/docs/en/tools-reference#lsp-tool-behavior)
- **PostToolUse verification hooks.** The canonical pattern is a `Write|Edit` PostToolUse hook
  that runs prettier/lint/tests and returns findings via `additionalContext` (or `decision:
  "block"`), so every edit is linted/typed automatically.
  [hooks guide](https://code.claude.com/docs/en/hooks)
- **Stop-hook done-ness gate.** A `Stop` hook that runs the test suite and blocks the agent from
  ending ("Tests failed: run `npm test`") turns "done" into a verifiable, external criterion
  rather than the model's self-judgment.
- **Correctness gates.** Read-before-edit and edit uniqueness prevent blind writes; `ReportFindings`
  and the code-review flow structure self-critique.
- Absent a Stop hook, **"done" = the model stops emitting tool calls**, bounded by the caps above.

## Ideas worth stealing

1. **A rich `PreToolUse` decision protocol (`allow`/`deny`/`ask`/`defer` + `updatedInput` +
   `additionalContext`).** BioRouter's permission inspectors are pass/prompt; Claude Code lets an
   *external* hook not just veto but **rewrite the tool's arguments and inject context** before
   execution. That single mechanism covers lab-policy enforcement (block writes outside a data
   dir), auto-fixups (normalize a shell command), and just-in-time context — without touching the
   Rust loop.

2. **An `auto`-mode-style action classifier with tool-results stripped.** A second, cheap model
   that vets each shell/network action, *seeing only user messages + tool calls + CLAUDE.md but
   not tool outputs*, is a strong prompt-injection defense and a far better UX than all-or-nothing
   autonomy. The **3-consecutive / 20-total block → fall back to prompting** rule is a clean,
   cheap stuck-loop circuit-breaker BioRouter could adopt almost verbatim.

3. **Structured compaction + re-injecting CLAUDE.md from disk.** Claude Code's summary keeps a
   *named schema* (intent, key concepts, files+snippets, errors→fixes, pending tasks, current
   work) and, crucially, **re-reads the project instructions from disk after compaction** instead
   of trusting them to survive the summary. Both are directly portable to BioRouter's
   `context_mgmt`.

4. **Tool-output offloading to files with a head-preview.** Capping tool output (~30 KB) and
   spilling the rest to a session file that the agent greps on demand keeps a giant SQL/grep/
   bioinformatics result from poisoning the window — a targeted, lossless alternative to
   summarizing everything. Cheap to implement in the shell tool.

5. **LSP after-edit auto-diagnostics as a built-in verification loop.** Reporting type errors/
   warnings automatically after every edit (no separate build) gives real, per-edit self-
   verification. For a Rust/R/Python research agent this closes the "did the edit actually
   compile/type-check" gap that otherwise relies on the model remembering to test.

6. **Background subagents that return only a summary.** Delegating research/exploration to an
   isolated-context worker that returns a 400-token summary instead of 6,000 tokens of file reads
   is the single biggest context-hygiene win, and the lightweight `Explore`/`Plan` agents (which
   skip CLAUDE.md) show how to make delegation cheap.

7. **Automatic checkpoints + one-command rewind.** Claude Code's edit checkpointing and rewind
   (plus worktree isolation for subagents) are exactly the safety net that Goose and BioRouter
   lacked when this report was written. A shadow snapshot before each edit and a `rewind` that
   restores files *and* conversation would move BioRouter ahead of Goose and make autonomous
   runs on a scientist's working tree much safer.

## Sources

Provenance is mixed by necessity: Claude Code is closed-source, so official documentation is
the primary source and reverse-engineering write-ups fill the gaps the docs deliberately
leave. Official sources are on `code.claude.com` and `platform.claude.com`; everything else
is third-party and marked as such in the sections above.

| Kind | Source |
|---|---|
| Official — memory and context files | [memory docs](https://code.claude.com/docs/en/memory) |
| Official — context window and compaction | [context-window docs](https://code.claude.com/docs/en/context-window), [compaction (platform)](https://platform.claude.com/docs/en/build-with-claude/compaction) |
| Official — tools | [tools reference](https://code.claude.com/docs/en/tools-reference) |
| Official — hooks | [hooks reference](https://code.claude.com/docs/en/hooks) |
| Official — permissions and sandboxing | [permission modes](https://code.claude.com/docs/en/permission-modes) |
| Official — subagents | [sub-agents](https://code.claude.com/docs/en/sub-agents) |
| Third-party — system prompt teardown | [dbreunig](https://www.dbreunig.com/2026/04/04/how-claude-code-builds-a-system-prompt.html) |
| Third-party — prompt analysis | [weaxs](https://weaxsey.org/en/articles/2025-10-12/) |
| Third-party — auto-compact threshold | [ClaudeLog](https://claudelog.com/faqs/what-is-claude-code-auto-compact/) |

## Related documentation

- [Goose report](goose.md) — Goose, the agent whose design influenced BioRouter most, and the agent whose gaps this report is measuring.
- [Gemini CLI report](gemini-cli.md) — the closest open-source analogue for hooks, policy and checkpointing, with source-level detail Claude Code cannot offer.
- [Cline report](cline.md) — an open-source agent that explicitly mirrors Claude Code's hook event names, useful for seeing the model implemented in readable source.
- [Safety and guardrails comparison](../../history/agent-loop-review/competitive-comparison/safety-and-guardrails.md) — the head-to-head chapter that scores these permission modes against the rest of the field.
- [Improvement proposals register](../../history/agent-loop-review/improvement-proposals.md) — the `BR-NN` index, including BR-6 (output offloading) and BR-9 (lower-trust project files).
