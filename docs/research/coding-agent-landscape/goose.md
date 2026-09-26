# Goose (Block / AAIF) — agentic feedback loop review

> **What this is.** An external review of Goose, the open source Rust AI agent originally
> by Block and now stewarded by the Agentic AI Foundation (AAIF, Linux Foundation), and the
> agent whose design influenced BioRouter most. It emphasises what Goose added or changed in
> 2025 and 2026 that BioRouter may not have. One of nine tool reports in this folder,
> each covering the same ten dimensions.
> **Status:** Current. It describes an external project, so BioRouter's own changes do not
> invalidate it, and it remains the repository's only detailed record of Goose's recent
> changes. It is a July 2026 snapshot of a fast moving project, so check every "gap compared
> with Goose" item again before acting on it.
> **Audience:** developers working on BioRouter's agent loop.

BioRouter is an independent project, but Goose was the strongest influence on its design,
and the two have a similar shape: a Rust core agent crate, a daemon (`goosed` in Goose,
`biorouterd` in BioRouter), a `ui/desktop` Electron app, hermit and a Justfile. That makes
this the one report in the folder about the design BioRouter learned most from rather than
about a competitor.

> **Convention.** Sections end with a **Gap compared with Goose:** callout naming the parts
> of that subsystem that Goose added in 2025 and 2026 and that BioRouter may not have. The
> callout appears only where such a gap was identified; a section without one is not a
> claim of parity, only that no gap was recorded.

> **Note.** Sources are primary where possible: the `block/goose` source tree (and its
> mirror `aaif-goose/goose`) on GitHub, plus the official docs at `goose-docs.ai` /
> `block.github.io/goose`. All URLs were fetched July 2026 — month granularity only, for a
> project that ships continuously. Goose file paths are cited without a commit or tag, so
> they cannot be traced back to an exact version.

## System prompt and context injection

Goose assembles the system prompt through a `PromptManager`
(`crates/goose/src/agents/prompt_manager.rs`). The base template is composed from the current
**permission mode** (the manager sets `is_autonomous=true` for Auto mode and injects
Chat-mode-specific instructions when tools are disabled), the enabled extensions/tools, and
project context files.
[DeepWiki: permission modes and tool approval](https://deepwiki.com/block/goose/6.2-permission-modes-and-tool-approval)

**Project context files.** Goose reads `AGENTS.md` first, then `.goosehints`, at *every
directory level* from the working dir up to the repo root, and — a 2025+ addition —
continues discovering nested hint files as it reads/writes files in subdirectories
(monorepo-friendly). The set of filenames is overridable via `CONTEXT_FILE_NAMES`. There is
also a **global** `~/.config/goose/.goosehints`. Hints are added to the system prompt on
*every request* (static). `@filename.md`/`@path` mentions inline a file's full content into
the immediate context rather than just referencing it.
[goosehints documentation](https://goose-docs.ai/docs/guides/context-engineering/using-goosehints/)

**Memory injection.** The built-in Memory extension loads all *global* memories
(`~/.config/goose/memory`) into system instructions at session start; *local* memories
(`.goose/memory`) are tag-retrieved on demand (see Compaction and memory).
[Memory MCP documentation](https://goose-docs.ai/docs/mcp/memory-mcp/)

> **Gap compared with Goose.** Hierarchical multi-level `AGENTS.md`/`.goosehints` discovery,
> `@`-mention inlining, and `CONTEXT_FILE_NAMES` are recent; older Goose versions loaded a
> single `.goosehints` at the repo root only.

## Tool loop mechanics

The core loop lives in `Agent::reply` / `reply_internal` (`crates/goose/src/agents/agent.rs`).
Each turn: the provider is streamed via `stream_response_from_provider` yielding
`(response, usage)` tuples; the loop checks a cancellation token per chunk. Tool requests in
the assistant message are split by `categorize_tools` into `frontend_requests` (handled by
`handle_frontend_tool_request`, streamed straight to the UI) and `remaining_requests` (real
MCP/extension tools — MCP is the Model Context Protocol).
[agent.rs source](https://raw.githubusercontent.com/block/goose/main/crates/goose/src/agents/agent.rs)

**Parallelism and streaming.** Remaining tools are first run through the permission
inspectors, then dispatched with `dispatch_tool_call`; the multiple tool futures are
multiplexed concurrently with `futures::select_all` and a `tokio::select! { biased; … }`
combined stream, so several tool calls in one assistant turn execute **in parallel** while
their partial outputs stream. Results are appended with `add_tool_response_with_metadata` and
fed back to the model on the next turn.

**Error handling and retries.** A `RetryManager` (`crates/goose/src/agents/retry.rs`) handles
provider errors with exponential backoff (retryable HTTP 429/5xx vs. fatal 401/403/422).
**Empty-turn retry:** if the model returns nothing, `empty_turn_retries` increments up to
`MAX_EMPTY_TURN_RETRIES = 3` before emitting `EMPTY_TURN_MESSAGE`. A failed tool result is
returned to the model as a tool error so it can self-correct.

**Turn cap:** `DEFAULT_MAX_TURNS = 1000` (env `GOOSE_MAX_TURNS`); `turns_taken` increments
each iteration (not on pure retries/stop-hook denials) and the loop breaks with
`MAX_TURNS_MESSAGE` when exceeded.
[environment variables documentation](https://github.com/aaif-goose/goose/blob/main/documentation/docs/guides/environment-variables.md)

## Compaction and memory

Context management is a first-class module: `crates/goose/src/context_mgmt/`. It is a
**two-tier** system (auto-compaction, then hard-limit fallbacks).
[smart context management](https://goose-docs.ai/docs/guides/sessions/smart-context-management/) ·
[context_mgmt/mod.rs source](https://raw.githubusercontent.com/block/goose/main/crates/goose/src/context_mgmt/mod.rs)

- **Auto-compaction.** Inside the loop, `check_if_compaction_needed` compares usage to
  `DEFAULT_COMPACTION_THRESHOLD = 0.8` (env `GOOSE_AUTO_COMPACT_THRESHOLD`; `0.0`
  disables). When tripped, the loop yields progress, calls `compact_messages` →
  `do_compact` (an LLM summarization driven by `crates/goose/src/prompts/compaction.md`),
  then `replace_conversation` swaps in the compacted history and continues **without user
  intervention**. What survives: recent user messages, a summary message (agent-visible),
  and continuation instructions; the *original* pre-compaction messages become
  user-visible-but-agent-invisible.
- **Background tool-output summarization.** Independently of full compaction, tool
  call/response pairs older than `GOOSE_TOOL_CALL_CUTOFF = 10` calls are condensed in the
  background (`maybe_summarize_tool_pairs`, `summarize_tool_call`,
  `TOOLCALL_SUMMARIZATION_BATCH_SIZE = 10`). This is a targeted "keep the last N tool
  outputs verbatim, shrink the rest" strategy distinct from whole-conversation summary.
- **Hard-limit fallback (`GOOSE_CONTEXT_STRATEGY`).** If a request still exceeds the
  window: `summarize`, `truncate` (drop oldest), `clear`, or `prompt` (ask user). Default
  is `prompt` interactively, `summarize` headless. `filter_tool_responses` progressively
  removes tool responses from the middle outward at 0→10→20→50→100%.
- **Manual:** `/summarize` compacts on demand.

**Cross-session memory** is the Memory MCP extension: tagged key/value notes in local
(`.goose/memory`) or global (`~/.config/goose/memory`) stores. Globals are injected at
startup; locals are pulled by tag when the user's request matches. Unlike static
`.goosehints`, memory is agent-writable ("remember that …") and read on demand.
[Memory MCP documentation](https://goose-docs.ai/docs/mcp/memory-mcp/)

> **Gap compared with Goose.** The 0.8 auto-compaction, background tool-pair summarization
> (`GOOSE_TOOL_CALL_CUTOFF`), and the pluggable `GOOSE_CONTEXT_STRATEGY` fallback ladder are
> all 2025 refinements; earlier Goose truncated more bluntly.

## Hooks and extensibility

Goose gained a full **lifecycle hooks** system in **May 2026**, its newest major loop
change.
[Goose hooks announcement](https://goose-docs.ai/blog/2026/05/14/goose-hooks/)

- **Events:** `SessionStart`, `SessionEnd`, `Stop`, `UserPromptSubmit`, `PreToolUse`,
  `PostToolUse`, `PostToolUseFailure`, `BeforeReadFile`, `AfterFileEdit`,
  `BeforeShellExecution`, `AfterShellExecution`.
- **Discovery:** follows the **Open Plugins** spec — any `~/.agents/plugins/<name>/hooks/
  hooks.json` (user scope) or `<project>/.agents/plugins/<name>/hooks/hooks.json` (project
  scope) is auto-loaded at startup. "A folder, a JSON file, a script. No registration, no
  daemon, no rebuild."
- **Config:** each event maps to matchers (regex tested against tool name / file path /
  shell command; omit to match all) and command hooks with a `${PLUGIN_ROOT}` variable.
- **Contract:** hooks receive a JSON payload on stdin (session id, tool name, cwd, …) and
  run external scripts. Failures/timeouts are logged but do not crash the host.

> **Note.** The docs are explicit that block/inject semantics mirror Claude Code's
> git-hook-style model. The safe reading is that `PreToolUse`/`Stop` can veto via exit code,
> but confirm against the Open Plugins spec before relying on injection.

The broader extensibility layer is **MCP**: extensions are stdio, Server-Sent Events (SSE),
or streamable-HTTP MCP servers providing tools/prompts/resources. The built-in **Developer**
extension supplies the core `shell` and `text_editor` tools.

## Guardrails and permissions

Four modes via the `GooseMode` enum, default **`smart_approve`** interactively (`auto`
headless/scheduled):
[permissions documentation](https://goose-docs.ai/docs/guides/goose-permissions/) ·
[DeepWiki: permission modes and tool approval](https://deepwiki.com/block/goose/6.2-permission-modes-and-tool-approval)

- **Auto** — every tool runs, no confirmation.
- **Approve** — every tool needs explicit Allow/Deny.
- **SmartApprove** — read-only tools auto-run; write/destructive tools prompt. An
  **LLM classifier** (`PermissionJudge`, template `permission_judge.md`) inspects tool
  name + arguments, uses an internal `platform__tool_by_tool_permission` tool to
  structure output, and `extract_read_only_tools` parses it (e.g. SQL `SELECT`, file
  reads, directory listings are auto-allowed).
- **Chat** — tools blocked entirely; conversational only.

Precedence: `GOOSE_MODE` env → `config.yaml` → runtime `/mode` command or Desktop toggle.
**Per-tool overrides** via `PermissionLevel` (`AlwaysAllow` / `AskBefore` / `NeverAllow`)
persisted in `permission.yaml` by a `PermissionManager`; extensions annotate tools with
`read_only` / `destructive` / `idempotent` hints. All tool calls flow through a
`ToolInspectionManager` of stacked inspectors before dispatch. **No OS-level sandboxing** is
documented — the guardrail is permission gating, not process isolation.

> **Gap compared with Goose.** SmartApprove plus the `PermissionJudge` LLM classifier and the
> annotation-driven per-tool `permission.yaml` are 2025 additions.

## Loop and stuck detection

- **`ToolInspectionManager`** runs inspectors before every tool dispatch (non-Chat mode).
  It includes a **`RepetitionInspector`** ("lower priority — basic repetition checking")
  that detects repeated identical tool calls, plus the permission inspector. This is the
  built-in guard against the model hammering the same tool.
  [agent.rs source](https://raw.githubusercontent.com/block/goose/main/crates/goose/src/agents/agent.rs)
- **Max turns** `GOOSE_MAX_TURNS` (default 1000) is the hard termination guarantee —
  external enforcement, not model self-restraint.
- **Empty-turn retry** cap `MAX_EMPTY_TURN_RETRIES = 3` catches a model that keeps
  returning nothing.
- **Stop hooks** can veto ending a turn; `emit_stop_hook_blocking` is capped at
  `DEFAULT_STOP_HOOK_BLOCK_CAP = 8` so a mis-behaving stop hook cannot loop forever.

> **Gap compared with Goose.** The `RepetitionInspector`, the empty-turn cap, and the
> stop-hook block cap are relatively recent safeguards against a stuck loop.

## Long-running tasks and background processes

- **Subagents** (Sept 2025). The lead agent can spawn independent subagent instances that
  run sub-tasks with **isolated context** ("keep your main conversation clean and
  focused"), inherit parent extensions unless customized, and run **in parallel or
  sequentially** (triggered by natural-language cues like "simultaneously" vs.
  "first…then"). Limits: `GOOSE_SUBAGENT_MAX_TURNS = 25`, 5-minute timeout, **cannot
  spawn nested subagents** (no infinite recursion) and cannot modify extensions. Results
  return as expandable `[subagent:N] tool | extension` entries, in full or summary mode.
  [subagents documentation](https://goose-docs.ai/docs/guides/context-engineering/subagents/)
- **Subrecipes vs subagents.** A *subrecipe* is a saved, parameterized Recipe invoked as a
  callable unit (predictable, reusable); a *subagent* is an ad-hoc delegated worker. Both
  give the lead/worker split.
  [recipes documentation](https://goose-docs.ai/docs/guides/recipes/)
- **Scheduler** (`crates/goose` scheduler): runs Recipes on **6-field cron** (auto-upgrades
  legacy 5-field), persisted as JSON in the data dir, surviving restarts; actions
  `add/list/remove/pause/unpause/update/run_now/sessions/kill/status`. Each fire creates a
  fresh headless session.
  [DeepWiki: scheduler and recurring tasks](https://deepwiki.com/block/goose/4.1.5-scheduler-and-recurring-tasks)
- **Lead/Worker model.** A two-model split (cheap worker + smart lead) was shipped in
  Aug 2025 (`GOOSE_LEAD_MODEL`, `GOOSE_LEAD_PROVIDER`) but has since been **removed and
  folded into Planning Mode / general multi-model config**, a good example of how quickly
  Goose changes: a design modelled on one release can be out of date a few months later.
  [lead/worker blog post, now marked removed](https://raw.githubusercontent.com/block/goose/main/documentation/blog/2025-08-11-llm-tag-team-lead-worker-model/index.md) ·
  [multi-model documentation](https://goose-docs.ai/docs/guides/multi-model/)

## State tracking and checkpoints

- **Plan mode** (CLI `/plan` … `/endplan`): Goose enters an interactive planning dialogue,
  asks clarifying questions, produces an actionable plan, then offers to **clear message
  history and act** — an explicit human checkpoint before edits. Uses a separate planner
  model via `GOOSE_PLANNER_PROVIDER` / `GOOSE_PLANNER_MODEL`.
  [creating plans documentation](https://goose-docs.ai/docs/guides/context-engineering/creating-plans/)
- **Recipes as durable plans**: structured YAML (instructions, prompt, extensions,
  parameters, sub-recipes, retry, response schema) — versionable, shareable state.
- **No native todo-list tool** (nothing equivalent to Claude Code's `TodoWrite`); progress
  tracking is emergent via plan mode + recipes.
- **No built-in git checkpoint / undo.** Unlike Claude Code's automatic edit checkpointing
  and `/rewind`, Goose has **no shadow-repo snapshot or one-command revert**. The
  documented pattern is a `.goosehints`/`AGENTS.md` instruction telling the agent to
  `git commit` after every change, turning git history into a manual undo stack.
  [community write-up of the undo pattern](https://dev.to/goose_oss/how-to-stop-your-ai-agent-from-making-unwanted-code-changes-5g85)
  This is a genuine gap versus newer coding agents (and an opportunity for BioRouter).

## Self-verification

Goose has no automatic "lint/test after every edit" loop in the core agent; verification is
opt-in through **Recipes**:
[recipe reference](https://goose-docs.ai/docs/guides/recipes/recipe-reference/) ·
[DeepWiki: subagents and tasks](https://deepwiki.com/block/goose/4.1.4-subagents-and-tasks)

- **`retry` + `SuccessCheck`.** A recipe can declare success checks — `shell` (run a
  command, e.g. tests/lint, and check exit code), `file` existence, or file-content regex.
  `handle_retry_logic` evaluates all checks; **all must pass** or, if retries remain, the
  configured `failure_prompt` is injected as a user message ("what went wrong") and the
  agent retries. This is a closed-loop test-then-fix mechanism.
- **`response.json_schema`.** A recipe can require the final output to validate against a
  JSON schema, forcing structured, checkable done-ness criteria.
- Otherwise "done" = model stops emitting tool calls (bounded by max-turns and stop hooks).

> **Gap compared with Goose.** Recipe `retry` / `SuccessCheck` / `response.json_schema` are
> the main self-verification machinery and are 2025-era; an agent without them relies purely
> on the model deciding it has finished.

## Ideas worth stealing

1. **Lifecycle hooks (Open Plugins spec).** The May-2026 hooks system (`PreToolUse`,
   `PostToolUse`, `AfterFileEdit`, `BeforeShellExecution`, `SessionStart/End`, `Stop`) makes
   the loop scriptable from *outside* the binary — no Rust, no MCP server. For BioRouter
   this is the cleanest way to add lab-specific policy (block writes outside a data dir,
   auto-run `cargo fmt`/tests after edits, audit-log every shell command) without forking
   the agent. Adopt the `<project>/.agents/plugins/*/hooks/hooks.json` discovery model.

2. **Recipe `retry` + `SuccessCheck` closed loop.** A declarative "run this shell check
   after the task; if it fails, re-inject a failure prompt and retry" gives real
   self-verification (tests pass, file exists, output matches schema) without hard-coding it
   in the loop. This is high-value for reproducible biomedical workflows where "the notebook
   actually runs / the figure was written" is the done-ness criterion.

3. **Background tool-output summarization (`GOOSE_TOOL_CALL_CUTOFF`).** Keeping only the
   last ~10 tool outputs verbatim and summarizing older ones *in the background* — separate
   from whole-conversation compaction — directly attacks the failure mode where a giant
   grep/SQL/bioinformatics tool result poisons the window. Cheaper and less lossy than
   full-history summarization.

4. **`RepetitionInspector` + inspector stack.** A pluggable pre-dispatch `ToolInspection
   manager` (repetition detection, permission checks) is a clean, testable place to add
   stuck-loop detection and dangerous-command classification. Fingerprint (tool name +
   args + result preview) and block the 3rd identical call — cheap insurance against
   token-burning oscillation, especially the search↔summarize loop.

5. **SmartApprove with an LLM `PermissionJudge` + tool annotations.** Auto-allowing
   read-only tools (annotated `read_only`, or classified live) while prompting on
   destructive ones removes most approval friction while keeping a human on write/exec.
   For a research agent hitting many read-only data queries, this is a major UX win over
   all-or-nothing autonomy.

6. **Native git checkpoint / rewind — the gap to close.** Goose *lacks* this and papers
   over it with "tell the model to commit." BioRouter could move ahead of Goose by adding a
   shadow repo snapshot before each edit and a `/rewind` that restores files + conversation
   — the single biggest safety-net difference between Goose and current-generation coding
   agents.

7. **Hierarchical `AGENTS.md`/context-file discovery + on-demand Memory extension.**
   Walking from cwd to repo root (and into subdirs as files are touched), plus a
   tag-retrieved, agent-writable Memory store, gives scoped context without bloating every
   prompt — a better fit for monorepo-style scientific codebases than one static hints file.

## Sources

Primary where possible: the `block/goose` source tree (and mirror `aaif-goose/goose`) on
GitHub, plus official documentation at `goose-docs.ai` / `block.github.io/goose`. DeepWiki
pages are a generated secondary source and are labelled as such below.

| Topic | Source |
|---|---|
| Agent loop, inspectors | [agent.rs](https://raw.githubusercontent.com/block/goose/main/crates/goose/src/agents/agent.rs) |
| Context management | [context_mgmt/mod.rs](https://raw.githubusercontent.com/block/goose/main/crates/goose/src/context_mgmt/mod.rs), [smart context management](https://goose-docs.ai/docs/guides/sessions/smart-context-management/) |
| Context files | [goosehints guide](https://goose-docs.ai/docs/guides/context-engineering/using-goosehints/) |
| Memory extension | [Memory MCP](https://goose-docs.ai/docs/mcp/memory-mcp/) |
| Hooks | [Goose hooks announcement](https://goose-docs.ai/blog/2026/05/14/goose-hooks/) |
| Permissions | [permissions guide](https://goose-docs.ai/docs/guides/goose-permissions/), DeepWiki (secondary): [permission modes](https://deepwiki.com/block/goose/6.2-permission-modes-and-tool-approval) |
| Environment variables | [environment-variables.md](https://github.com/aaif-goose/goose/blob/main/documentation/docs/guides/environment-variables.md) |
| Subagents and recipes | [subagents guide](https://goose-docs.ai/docs/guides/context-engineering/subagents/), [recipes guide](https://goose-docs.ai/docs/guides/recipes/), [recipe reference](https://goose-docs.ai/docs/guides/recipes/recipe-reference/) |
| Scheduler | DeepWiki (secondary): [scheduler and recurring tasks](https://deepwiki.com/block/goose/4.1.5-scheduler-and-recurring-tasks) |
| Lead/worker and multi-model | [lead/worker blog post](https://raw.githubusercontent.com/block/goose/main/documentation/blog/2025-08-11-llm-tag-team-lead-worker-model/index.md), [multi-model guide](https://goose-docs.ai/docs/guides/multi-model/) |
| Plan mode | [creating plans guide](https://goose-docs.ai/docs/guides/context-engineering/creating-plans/) |
| Undo pattern | [community write-up](https://dev.to/goose_oss/how-to-stop-your-ai-agent-from-making-unwanted-code-changes-5g85) |

> **Note.** Goose file paths are cited without a commit or tag, so they cannot be traced back
> to an exact version. Check any "gap compared with Goose" item again before acting on it.

## Related documentation

- [Claude Code report](claude-code.md) — the checkpoint/rewind safety net this report identifies as Goose's largest gap.
- [Gemini CLI report](gemini-cli.md) — a from-scratch competitor whose loop detection and policy engine are the concrete blueprints for the gaps named here.
- [Cline report](cline.md) — the shadow-git checkpoint model, described in readable source.
- [Agent-loop campaign](../../history/agent-loop-campaign/README.md) — the implementation campaign that acted on the gaps this report identified.
- [Improvement proposals register](../../history/agent-loop-review/improvement-proposals.md) — the `BR-NN` index of proposals derived from this corpus.
- [Context engineering guide](../../agent-loop/context-engineering.md) — how BioRouter's context management works today.
