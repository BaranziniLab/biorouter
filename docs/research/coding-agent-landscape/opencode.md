# OpenCode (sst) — agentic feedback loop review

> **What this is.** An external review of how OpenCode — "the open source AI coding agent,"
> built by SST as a TypeScript/Bun monorepo — implements its agentic loop, with emphasis on
> its client/server split, private git-object-DB snapshots with `/undo` and `/redo`, and
> prune-before-summarize compaction. One of eight tool reports in this folder, each covering
> the same ten dimensions.
> **Status:** Current. External-tool research; it is the cited source for the private
> git-object-DB checkpoint approach adopted in proposal BR-43. Research date 2026-07-12.
> **Audience:** developers working on BioRouter's agent loop.

`BR-NN` identifiers name proposals in the agent-loop review's improvement register; the
index lives in [the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

OpenCode's architectural spine is a **client/server split**: the core agent logic runs as a
local **Hono-based HTTP server** that owns sessions, tool execution, provider calls and
state; multiple clients (Go TUI, desktop app, VS Code extension, web) connect over HTTP +
**Server-Sent Events (SSE)**
([DeepWiki: core architecture](https://deepwiki.com/sst/opencode/2-architecture)). State
persists in **SQLite via Drizzle ORM**; the OpenAPI 3.1 spec in `packages/sdk/openapi.json`
generates typed SDKs with `@hey-api/openapi-ts`
([DeepWiki search](https://deepwiki.com/sst/opencode)). Server as the single source of truth,
thin clients subscribing to an event bus — every loop mechanic below hangs off that.

> **Note — repository identity.** The canonical repository has been mirrored and renamed
> across `sst/opencode` and `anomalyco/opencode` during OpenCode's history. Issue and PR
> citations below use whichever org the tracker returned, so a link may point at either;
> both refer to the same project.

> **Note — sourcing.** Primary sources are opencode.ai docs and the `sst/opencode` repo.
> Several architectural claims rest on **DeepWiki**, a generated secondary source; those
> claims carry a DeepWiki citation inline, and should be treated as high-but-not-source-
> verified. At 84 lines this report covers the same ten dimensions that
> [claude-code.md](claude-code.md) covers in nearly 300, so each section here is a single
> dense paragraph — thin coverage is a property of this report, not of the feature.

## System prompt and context injection

OpenCode assembles the system prompt in a fixed stack, orchestrated by
`packages/opencode/src/session/prompt.ts`
([prompt-construction deep dive](https://gist.github.com/rmk40/cde7a98c1c90614a27478216cc01551f)):

1. **Provider-specific base prompt** — a static `.txt` chosen by model id: Claude gets `anthropic.txt`, GPT/o-series get `beast.txt`, Gemini gets `gemini.txt`, etc. An agent's own prompt, if defined, *replaces* the provider file entirely.
2. **Environment block** (generated per call by `system.ts`): working directory, "Is directory a git repo," platform, model name, and today's date — plus a project file-structure listing ([open-docs system prompts](https://github.com/bgauryy/open-docs/blob/main/docs/opencode/05-system-prompts.md)).
3. **Instruction files** (`instruction.ts`): OpenCode's context-file convention is **`AGENTS.md`**, with **`CLAUDE.md`/`~/.claude/CLAUDE.md` as a migration fallback**, plus `CONTEXT.md`. It walks from the working dir up to the worktree root, then checks the global `~/.config/opencode/AGENTS.md`, first match wins per tier ([Rules docs](https://opencode.ai/docs/rules/)). Extra files come from the `instructions` config key, which also supports **remote URLs fetched with a 5-second timeout**. Each file is prefixed `Instructions from: <path>`.

Tool descriptions are mostly static `.txt` too, but four use runtime substitution: `bash`
(interpolates dir/maxLines/maxBytes), `task` (`{agents}` → available subagents), `skill` (an
XML block listing each discovered skill's name/description/file), and `websearch`
(`{{year}}`).

The most interesting mechanism is **mid-conversation injection**: when the `read` tool opens
a file in a subdirectory, OpenCode walks *up from that file's directory* looking for
instruction files not already loaded and injects them into the tool output as
`<system-reminder>` blocks
([prompt gist](https://gist.github.com/rmk40/cde7a98c1c90614a27478216cc01551f)). Project
rules thus arrive lazily, scoped to where the agent is actually working, rather than all
up-front.

## Tool loop mechanics

The server's `SessionProcessor`/`LLM.stream` runs the reasoning loop: it calls the provider
via `streamText()`, streams assistant output as **typed "parts"** (text, reasoning, tool-call,
file) that decompose each message, and re-enters the loop after tool results
([DeepWiki: architecture](https://deepwiki.com/sst/opencode/2-architecture)). Everything
streams to clients over SSE, so the TUI/desktop render tokens and tool output live. Built-in
tools: `bash`, `edit`, `write`, `read`, `grep`, `glob`, `list`, `apply_patch`, `lsp`
(experimental), `skill`, `todowrite`, `webfetch`, `websearch` (Exa, gated by
`OPENCODE_ENABLE_EXA`), `question`, and `task` ([Tools docs](https://opencode.ai/docs/tools/)).
`ProviderTransform` normalizes messages across Anthropic/OpenAI/Gemini so tools work
vendor-agnostically.

**Parallelism is limited.** Multiple `task` (subagent) calls in one assistant turn execute
*sequentially* — the session loop does `tasks.pop()`, awaits it, then continues — a known
limitation ([issue #14195](https://github.com/anomalyco/opencode/issues/14195)). Error
handling includes `repairToolCall` for malformed/truncated calls (a `finishReason: length`
truncation is a documented failure mode,
[issue #18108](https://github.com/anomalyco/opencode/issues/18108)).

## Compaction and memory

Compaction lives in `packages/opencode/src/session/compaction.ts` with overflow detection in
`processor.ts`/`overflow.ts`
([DeepWiki: context management and compaction](https://deepwiki.com/sst/opencode/2.4-context-management-and-compaction)).
Rather than one blunt threshold, OpenCode layers three strategies:

- **Overflow trigger:** `isOverflow` fires when tokens exceed *usable* context = total window − reserved output (default 32,000) − a `COMPACTION_BUFFER` safety margin of 20,000. Auto-compaction is on by default; `OPENCODE_DISABLE_AUTOCOMPACT` turns it off ([badlogic compaction research](https://gist.github.com/badlogic/cd2ef65b0697c4dbe2d13fbecb0a0a5f)).
- **Tool-output pruning first** (`SessionCompaction.prune`): scans newest→oldest, **protects the last 2 turns** (`DEFAULT_TAIL_TURNS`), only prunes when total tool output exceeds 40k tokens (`PRUNE_PROTECT`) and at least 20k (`PRUNE_MINIMUM`) can be reclaimed, and **never prunes `skill` outputs**. This reclaims space by dropping verbose tool bodies while keeping the execution record (names/params).
- **Structured summarization** as a fallback: a `SUMMARY_TEMPLATE` drives a summary agent to produce a *continuation-oriented* markdown summary with sections Goal / Constraints / Progress / Key Decisions / Next Steps / Critical Context. Notably, a **"Session Memory Compact"** path reuses structured info already in session memory to *avoid an LLM call* for most auto-compactions.

Media (images/PDFs) is stripped to text placeholders (`[Attached image/jpeg: filename.jpg]`)
under context pressure. After compaction the last user message is replayed (hard overflow) or
a synthetic "Continue if you have next steps" prompt is appended (proactive). Cross-session
memory is via **AGENTS.md** (persistent project/global rules) rather than an evolving memory
store.

## Hooks and extensibility

**Plugins** are JS/TS modules —
`export const MyPlugin = async ({ project, client, $, directory, worktree }) => ({ ...hooks })`
— loaded from `.opencode/plugins/`, `~/.config/opencode/plugins/`, or npm packages in
`opencode.json` (Bun auto-installs deps into `~/.cache/opencode/node_modules/`)
([Plugins docs](https://opencode.ai/docs/plugins/)).

The hook/event surface is broad: `tool.execute.before`/`tool.execute.after`; session lifecycle
(`session.created/updated/compacted/deleted/error`); `file.edited`, `file.watcher.updated`;
`message.updated/removed`, `message.part.updated`; `permission.asked`/`permission.replied`;
`lsp.client.diagnostics`, `shell.env`; and TUI hooks (`tui.prompt.append`,
`tui.command.execute`, `tui.toast.show`). Plugins can **block** (throw to reject an operation
— e.g. deny `.env` reads), **inject/mutate** tool args, env vars, or session context, and
**register custom tools** with schema + execute. OpenCode also supports **MCP (Model Context
Protocol) servers** and
**ACP** (Agent Communication Protocol) for external tool/agent integration.

## Guardrails and permissions

Permissions are the central guardrail, keyed by tool class: `read`, `edit`, `bash`,
`webfetch`, `websearch`, `external_directory`, `task`, `skill`, `lsp`, `question`, `glob`,
`grep`, and `doom_loop`, each resolving to **`allow` / `ask` / `deny`**
([Permissions docs](https://opencode.ai/docs/permissions/)). Bash (and other keys) support
**wildcard pattern rules** where the **last matching rule wins**:

```json
{ "permission": { "bash": { "*": "ask", "git *": "allow", "rm *": "deny" } } }
```

Defaults are mostly `allow`, but **`doom_loop` and `external_directory` default to `ask`, and
`.env` files default to `deny`.** Approval prompts offer `once` / `always` (session-scoped for
matching pattern) / `reject`. **Per-agent overrides** narrow the global policy — e.g. the
built-in **Plan agent** sets all edits and all bash to `ask`, giving a read-only-by-default
posture ([Agents docs](https://opencode.ai/docs/agents/)). Permission keys are matched as
wildcard patterns against the underlying tool name, so the same syntax covers built-ins,
custom tools, and MCP tools. Sandboxing is coarse (the `external_directory` gate scopes access
to the workspace); OpenCode leans on the permission prompts + Plan mode rather than OS
sandboxing.

## Loop and stuck detection

OpenCode ships **explicit doom-loop detection** — a first-class mechanism BioRouter lacks.
Implemented in `session/prompt.ts`, it keeps a Map of `tool + JSON.stringify(input)` counts
and escalates in stages ([PR #3445](https://github.com/sst/opencode/pull/3445),
[issue #25254](https://github.com/anomalyco/opencode/issues/25254)):

- **Stage 1 (3 identical calls):** inject a **non-blocking warning** nudging the model to try a different approach.
- **Stage 2 (5 identical calls):** escalate to the **`doom_loop` permission** (defaults to `ask`), pausing for user approval or failing.

It is deliberately **model-agnostic** (re-worked from a model-specific version because aliases
like GLM-4.6/Grok slipped through). Scenarios it targets: repeated identical searches,
re-reading the same file expecting different content, retrying the same failing patch, or
re-running a command hoping for different output. There are ongoing refinements for catching
repetition during reasoning/output blocks
([issue #12716](https://github.com/anomalyco/opencode/issues/12716)) and for cross-message
repetitions. Beyond doom-loop, the overflow/compaction machinery caps runaway context growth,
and subagents carry TTLs (below).

## Long-running tasks and background processes

Delegation is via the **`task` tool**, which spins up a **subagent in a child session**
([Agents docs](https://opencode.ai/docs/agents/)). Built-in subagents: **General**
(full-access, multi-step), **Explore** (read-only fast codebase search), **Scout** (read-only
external docs/deps). Which subagents a primary agent may spawn is gated by `permission.task`
glob patterns.

**Background subagents** are experimental — behind
`OPENCODE_EXPERIMENTAL_BACKGROUND_SUBAGENTS=true`, `task(background=true, ...)` runs an
independent session with a **30-minute TTL**; the parent can only `task_status(task_id)` for
running/completed/cancelled state, with **no streaming of intermediate output** (an open gap,
[issue #27828](https://github.com/anomalyco/opencode/issues/27828)). As noted, multiple task
calls in one turn still run one-at-a-time. Bash commands run through the server with
configurable output caps (maxLines/maxBytes); scheduling/cron is not a core feature.

## State tracking and checkpoints

Two complementary systems.

**Todo lists**: the `todowrite` tool manages a task list to track progress on complex
operations, surfaced in the UI.

**Snapshots / undo** is the standout ([Undo docs](https://v2.opencode.ai/snapshots)):
OpenCode captures the worktree **before and after each model step**, recording changed paths
on the assistant message, using **a separate internal Git object database in OpenCode's data
dir — it does *not* create commits, move branches, or touch your repo's Git index**. Tracked +
non-ignored untracked files are captured (untracked files >2 MiB excluded). `/undo` hides the
selected user message and all later ones (not deleted), drops the message text back into the
composer for revision, and **restores changed files to their earlier contents (deleting files
that didn't previously exist)**; repeated `/undo` walks the boundary earlier while keeping a
redo baseline. `/redo` un-hides messages and restores files to their pre-undo state. Exposed
programmatically as `client.session.revert()` / `unrevert()`. Caveat: undo can't reverse
external side effects (DB writes, shell effects), and there are known edge-case bugs in
file-vs-message restoration
([issue #27664](https://github.com/anomalyco/opencode/issues/27664)). **Plan mode** (the Plan
agent) is the read-only "analyze first, don't edit" state.

## Self-verification

OpenCode's primary verification loop is **LSP-diagnostics-as-feedback**: 25+ built-in
Language Server Protocol (LSP) servers auto-detect by file extension and start on demand
(given deps like a Go compiler or .NET SDK); after edits the agent receives **language-server diagnostics** (type errors, lint
issues) as structured feedback it can act on ([LSP docs](https://opencode.ai/docs/lsp/)). The
experimental `lsp` tool also exposes definitions/references/hover/call-hierarchy for
"surroundings awareness."

Complementing this, **formatters run automatically after every write/edit** — checking file
extension against enabled formatters (Prettier, gofmt, rustfmt/cargofmt, etc.) and applying
changes in the background ([Formatters docs](https://opencode.ai/docs/formatters/)); they're
opt-in (`"formatter": true`). "Done-ness" is mostly model-judged against AGENTS.md conventions
rather than an enforced test gate — there's no built-in mandatory test-after-edit loop, though
the `session.error` hook and diagnostics give plugins/agents the signals to build one.

## Ideas worth stealing

1. **Doom-loop detection with staged escalation.** A Map of `(tool + JSON.stringify(args))` counts, warning-inject at 3 repeats and permission-gate at 5, is cheap (O(1)) and model-agnostic. BioRouter's agent loop should add exactly this — it directly attacks the "agent re-runs the same failing edit/search" failure that wastes tokens and frustrates users ([PR #3445](https://github.com/sst/opencode/pull/3445)).

2. **Git-object-DB snapshots decoupled from the user's repo.** Capturing worktree state per model step into a *private* Git object database — no commits, no branch moves, no index changes — gives reliable `/undo`/`/redo` of agent edits without polluting the user's history. This is a much safer checkpoint model than committing to the real repo ([Undo docs](https://v2.opencode.ai/snapshots)).

3. **Layered compaction: prune tool outputs before summarizing, and skip the LLM when possible.** Protecting the last N turns, only pruning verbose tool bodies above token thresholds, exempting critical (skill) outputs, and a "Session Memory Compact" path that reuses structured state to avoid an LLM summary call — this is far more surgical (and cheaper) than one summarize-everything pass ([DeepWiki compaction](https://deepwiki.com/sst/opencode/2.4-context-management-and-compaction)).

4. **Lazy, directory-scoped context injection via `<system-reminder>`.** Instead of front-loading every rule file, inject the nearest unseen AGENTS.md when the agent reads a file in that subtree. This keeps the base prompt lean and delivers project rules exactly when relevant ([prompt gist](https://gist.github.com/rmk40/cde7a98c1c90614a27478216cc01551f)).

5. **Wildcard permission rules with last-match-wins + per-agent overrides.** A single pattern grammar (`"git *": "allow"`, `"rm *": "deny"`) spanning built-ins, custom, and MCP tools, layered global→agent, is expressive yet simple, and secure-by-default for `.env`/external dirs. It's a clean model for BioRouter's permission modes ([Permissions docs](https://opencode.ai/docs/permissions/)).

6. **LSP diagnostics + auto-formatters as a passive verification loop.** Feeding language-server diagnostics back into the agent after edits (and auto-formatting writes) gives cheap correctness signals without a bespoke test harness — a natural fit for BioRouter's multi-language research code ([LSP docs](https://opencode.ai/docs/lsp/), [Formatters docs](https://opencode.ai/docs/formatters/)).

7. **Client/server split with an SSE event bus + typed message "parts."** Making the agent a headless server that many thin clients subscribe to (and generating typed SDKs from an OpenAPI spec) is exactly BioRouter's `biorouterd` direction; OpenCode's part-based message model and SyncEvent replay are a proven reference for streaming and multi-client session sharing ([DeepWiki architecture](https://deepwiki.com/sst/opencode/2-architecture)).

## Sources

Primary: [opencode.ai docs](https://opencode.ai/docs/tools/) and the `sst/opencode`
repository. Secondary: DeepWiki's source-derived architecture pages, and community
source deep-dives. Every claim is cited inline; where docs were thin, source-file references
were fetched via DeepWiki or community write-ups and flagged at the point of use.

| Kind | Source |
|---|---|
| Docs — tools, rules, agents | [Tools](https://opencode.ai/docs/tools/), [Rules](https://opencode.ai/docs/rules/), [Agents](https://opencode.ai/docs/agents/) |
| Docs — permissions, plugins | [Permissions](https://opencode.ai/docs/permissions/), [Plugins](https://opencode.ai/docs/plugins/) |
| Docs — verification | [LSP](https://opencode.ai/docs/lsp/), [Formatters](https://opencode.ai/docs/formatters/) |
| Docs — snapshots and undo | [Undo / snapshots](https://v2.opencode.ai/snapshots) |
| Secondary — architecture | DeepWiki: [core architecture](https://deepwiki.com/sst/opencode/2-architecture), [context management and compaction](https://deepwiki.com/sst/opencode/2.4-context-management-and-compaction) |
| Community — prompt construction | [rmk40 gist](https://gist.github.com/rmk40/cde7a98c1c90614a27478216cc01551f), [open-docs system prompts](https://github.com/bgauryy/open-docs/blob/main/docs/opencode/05-system-prompts.md) |
| Community — compaction research | [badlogic gist](https://gist.github.com/badlogic/cd2ef65b0697c4dbe2d13fbecb0a0a5f) |
| Issues and PRs | [#3445](https://github.com/sst/opencode/pull/3445), [#12716](https://github.com/anomalyco/opencode/issues/12716), [#14195](https://github.com/anomalyco/opencode/issues/14195), [#18108](https://github.com/anomalyco/opencode/issues/18108), [#25254](https://github.com/anomalyco/opencode/issues/25254), [#27664](https://github.com/anomalyco/opencode/issues/27664), [#27828](https://github.com/anomalyco/opencode/issues/27828) |

> **Note.** Issue and PR links resolve under either `sst/opencode` or `anomalyco/opencode`
> depending on when they were captured; see the repository-identity note at the top.

## Related documentation

- [Cline report](cline.md) — the shadow-git checkpoint alternative, with a three-axis restore split instead of undo/redo.
- [Gemini CLI report](gemini-cli.md) — the deepest report in this folder, useful where a section here is a single paragraph.
- [Claude Code report](claude-code.md) — the same ten dimensions at maximum depth, for a fuller picture of any thin section here.
- [Shadow-git checkpoints design](../../agent-loop/designs/shadow-git-checkpoints.md) — BR-43, the BioRouter design that adopted this private-object-DB approach.
- [System overview](../../architecture/system-overview.md) — BioRouter's own client/server split, the direction OpenCode's architecture validates.
