# Pi (badlogic / Mario Zechner) — agentic feedback loop review

> **What this is.** An external review of Pi's deliberately minimal agent loop — a
> ~1000-token system prompt, no MCP, no subagents, session-tree branching, and typed
> extension hooks that can rewrite the prompt and the message array. One of eight tool
> reports in this folder, each covering the same ten dimensions.
> **Status:** Current. External-tool research; the source for the per-directory Project
> Trust idea cited by proposal BR-9 and for the "split turn" compaction fallback in BR-11.
> **Audience:** developers working on BioRouter's agent loop.

`BR-NN` identifiers name proposals in the agent-loop review's improvement register; the
index lives in [the improvement proposals register](../../history/agent-loop-review/improvement-proposals.md).

Pi's thesis is *subtractive*: "what you leave out matters more than what you put in." The
core is deliberately tiny — a ~1000-token system prompt, a handful of tools, no MCP (the
Model Context Protocol), no plan
mode, no built-in todos, no sub-agents, no permission popups, no background bash — and
everything workflow-specific is pushed into extensions, skills, and external tools like tmux
and containers
([usage.md, "Design Principles"](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/usage.md)).

## Names, packages and repositories

Pi has been renamed more than once; the table disambiguates the identifiers used throughout
this report.

| Thing | Current | Formerly |
|---|---|---|
| npm package | `@earendil-works/pi-coding-agent` | `@mariozechner/pi-coding-agent` |
| Monorepo | `earendil-works/pi-mono` | `badlogic/pi-mono` |
| Author | Mario Zechner | handle "badlogic," creator of libGDX |

Source links below still resolve under `badlogic/pi-mono`.

> **Provenance.** Two source families are in play and they disagree in places. The **design
> blog** (2025-11-30) describes an earlier state — it says "no compaction yet" — while the
> **shipped codebase** has since grown auto-compaction, parallel tools, and session trees.
> This report reflects the current docs and source, and flags where the codebase evolved past
> the blog. Claims sourced to the blog carry a `design blog` citation and should be read as
> design rationale, not as a description of current behaviour.

> **Note.** This report does not record the date its research was performed, unlike the
> [Cline](cline.md) and [OpenCode](opencode.md) reports, which both carry 2026-07-12.

## System prompt and context injection

Pi has "the shortest system prompt of any agent that I'm aware of," in the words of Armin
Ronacher, whose independent analysis is cited in Sources below. It is roughly:
"You are an expert coding assistant..." plus one-line tool descriptions, kept "below 1000
tokens" including tool definitions
([design blog](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/)).

Users can replace it wholesale with `.pi/SYSTEM.md` (project) or `~/.pi/agent/SYSTEM.md`
(global), append to it with `APPEND_SYSTEM.md`, or override per-run via `--system-prompt` /
`--append-system-prompt`
([usage.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/usage.md)).

Project context comes from `AGENTS.md` **or** `CLAUDE.md` files, loaded at startup from
`~/.pi/agent/` (global), then every parent directory walking up to cwd, then cwd itself —
hierarchical, most-specific last. Disable with `--no-context-files`. Context files load
*regardless of project trust* (unlike settings/extensions)
([security.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/security.md)).
Skills are advertised in the system prompt as XML name+description pairs (progressive
disclosure), and the full `SKILL.md` is only `read` on demand
([skills.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/skills.md)).

Mid-conversation injection is a first-class extension capability:

- The **`before_agent_start`** event (fires after the user submits, before the agent loop) lets an extension **inject a persistent message** into the session and/or **rewrite the system prompt for that turn**, chained across extensions. It exposes `systemPromptOptions` (selected tools, tool snippets, guideline bullets, context files, skills, cwd) so extensions edit the prompt without re-deriving it.
- The **`context`** event fires before *every* LLM call and hands the extension a deep copy of the message array to filter or modify non-destructively.

([extensions.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md))

## Tool loop mechanics

The blog's "four tools" (read, write, edit, bash) is the philosophical core; the shipped CLI
ships seven built-ins
([usage.md tool options](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/usage.md)):

| Tool | Behaviour |
|---|---|
| `read` | Defaults to the first 2000 lines, with offset/limit |
| `bash` | Runs synchronously with an optional timeout, returning stdout + stderr |
| `edit` | Exact-match `oldText` replacement |
| `write` | Auto-creates parent dirs |
| `grep` | Search |
| `find` | Search |
| `ls` | Listing |

Tool arguments are validated with TypeBox + AJV schemas producing detailed validation errors,
and tools separate LLM-facing content from UI-display content, with partial-JSON streaming so
diffs render as they arrive
([design blog](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/)).

**Parallelism evolved.** The blog said the loop is synchronous; the current codebase has a
**default parallel tool-execution mode**: sibling tool calls from one assistant message are
"preflighted sequentially, then executed concurrently." Lifecycle events reflect this —
`tool_execution_start` fires in assistant source order during preflight,
`tool_execution_update` interleaves, `tool_execution_end` fires in *completion* order, and
final `toolResult` messages are re-emitted in source order
([extensions.md tool events](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md)).

Streaming is architecturally decoupled: `AssistantMessageStream` separates provider
Server-Sent Events (SSE) / websocket transport reads from downstream consumption, so the
harness can await hooks,
persistence, and save-points without stalling the transport reader
([agent-harness.md](https://github.com/badlogic/pi-mono/blob/main/packages/agent/docs/agent-harness.md)).
Error handling uses `Result<TValue, TError>` for expected failures (shell, filesystem,
resource loading, compaction) that "must not throw"; public harness failures normalize to
`AgentHarnessError` preserving the subsystem error as `cause`. Provider streams are **not
resumable**, and "unfinished tool calls are unsafe to retry unless tools declare
idempotent/retry-safe behavior."

## Compaction and memory

Pi now ships full auto-compaction plus branch summarization
([compaction.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/compaction.md)).
Auto-compaction triggers when `contextTokens > contextWindow − reserveTokens` (default
`reserveTokens` = 16384), or manually via `/compact [instructions]`.

The algorithm: walk backwards accumulating tokens until `keepRecentTokens` (default 20000) is
preserved; summarize everything older with an LLM call; append a
`CompactionEntry {summary, firstKeptEntryId, tokensBefore, details}`; reload the session as
`system prompt + summary + messages from firstKeptEntryId onward`. Cut points are only allowed
at user/assistant/bash/custom messages — **never** between a tool call and its result. A
single oversized turn produces a "split turn" that cuts mid-turn and generates two merged
summaries (history + turn-prefix).

The structured summary format is a fixed markdown template — Goal / Constraints / Progress
(Done, In Progress, Blocked) / Key Decisions / Next Steps / Critical Context — plus
`<read-files>` and `<modified-files>` blocks. File operations are tracked **cumulatively**
across successive compactions and nested branch summaries. Before summarizing, messages are
serialized to a non-conversational transcript (`[User]:`, `[Assistant tool calls]:`,
`[Tool result]:`) so the model doesn't try to continue it, and tool results are truncated to
2000 chars with a marker.

Extensions can fully override compaction via `session_before_compact` (cancel, or supply a
custom summary generated by a different/cheaper model). Cross-session memory is file-based:
users keep `TODO.md`/`PLAN.md`, and sessions persist to disk (see State tracking and
checkpoints).

## Hooks and extensibility

This is Pi's richest surface. Extensions are TypeScript modules (loaded via jiti, no build
step) auto-discovered from `~/.pi/agent/extensions/` and (after trust) `.pi/extensions/`, or
installed as npm/git "pi packages." An extension is a default-export factory receiving
`pi: ExtensionAPI`; it can `registerTool`, `registerCommand` (`/mycommand`),
`registerShortcut`, `registerFlag`, `registerProvider`, render custom TUI, and persist state
with `pi.appendEntry()`
([extensions.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md)).

The event catalog is large and lifecycle-ordered: startup (`project_trust`,
`resources_discover`), session (`session_start`, `session_before_switch/fork/compact/tree`,
`session_compact`, `session_tree`, `session_shutdown`), agent (`before_agent_start`,
`agent_start/end/settled`, `turn_start/end`, `message_start/update/end`), model
(`model_select`, `thinking_level_select`), provider (`before_provider_headers`,
`before_provider_request`, `after_provider_response`), tool (`tool_execution_start/update/end`,
`tool_call`, `tool_result`), `context`, `input`, and `user_bash`.

What they can do:

- **`tool_call` can block** (`{block: true, reason}`) *and* mutate `event.input` in place (no re-validation) before execution.
- **`tool_result` chains like middleware**, each handler patching content/details/isError.
- **`context` and `before_provider_request` can rewrite the entire outgoing payload**, including system instructions.
- **`message_end` can replace a finalized message** (same role); `before_provider_headers` can add/delete headers.

Ronacher's key observation: you don't download a capability, "you ask the agent to extend
itself" — and Pi supports hot reload (`/reload`) so the agent can "write code, reload, test it
and go in a loop until [the] extension actually is functional."

## Guardrails and permissions

Pi is deliberately **YOLO by default**: "unrestricted access to your filesystem and can
execute any command without permission checks or safety rails"
([design blog](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/)). The security doc
is explicit that there is **no built-in sandbox**: built-in tools and extensions run with the
full permissions of the launching user, and "a partial in-process sandbox would be easy to
misunderstand as a security boundary." Real isolation must come from the OS/VM/container
([security.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/security.md)).

The one built-in guard is **Project Trust** — an *input-loading* gate, not a runtime sandbox.
On first entry to a directory containing `.pi/settings.json`,
`.pi/extensions|skills|prompts|themes`, `.pi/SYSTEM.md`, or `.agents/skills`, Pi asks before
loading those project resources (`defaultProjectTrust: ask|always|never`, decisions saved to
`~/.pi/agent/trust.json`). Context files (`AGENTS.md`/`CLAUDE.md`) load regardless — prompt
injection from repo files "is expected local-agent risk and cannot be reliably prevented."

Approval-style gating is opt-in via extensions (the docs' canonical example is a `tool_call`
handler that `ctx.ui.confirm()`s before `rm -rf`/`sudo`). Read-only operation is achieved by
tool allow-listing: `pi --tools read,grep,find,ls`. Recommended isolation patterns
(`containerization.md`): whole-process container, or host Pi routing built-in tool execution
into a **Gondolin** micro-VM, or **OpenShell** policy sandbox.

## Loop and stuck detection

There is essentially none by design. The blog states the loop has "no maximum steps or similar
knobs" and runs "until the agent says it's done." No repetitive-call detection, no error-streak
breaker, no max-iteration cap are documented.

The harness does implement **context-overflow recovery**: on overflow it auto-compacts and
*retries* the aborted turn (`session_before_compact` exposes `reason: "overflow"` and
`willRetry`), and `agent_end` notes Pi "may still auto-retry, auto-compact and retry, or
continue with queued follow-up messages" before `agent_settled` fires
([extensions.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md),
[agent-harness.md](https://github.com/badlogic/pi-mono/blob/main/packages/agent/docs/agent-harness.md)).
The harness doc lists retry-decision-point handling as **not yet implemented** ("Implement
retry handling").

Human steering is the intended stuck-breaker:

- `Enter` queues a steering message delivered after the current tool batch.
- `Alt+Enter` queues a follow-up after all work.
- `Escape` aborts the run, clearing steering/follow-up queues but preserving `nextTurn` messages.

## Long-running tasks and background processes

Intentionally omitted. `bash` runs **synchronously** — "no built-in way to start a dev server,
run tests in the background, or interact with a REPL." The recommended pattern is **tmux**:
the agent starts a process in a named tmux session and polls it, giving observability the user
shares (there's a dedicated tmux doc).

Sub-agents and delegation are also intentionally not built in ("no dedicated sub-agent tool");
the author discourages mid-session context-gathering sub-agents in favor of file-based
artifacts, but a sub-agent can be spawned by having the agent invoke `pi` itself via bash
(e.g. a `/review` slash command). There is a separate, explicitly **experimental**
`@earendil-works/pi-orchestrator` package ("may change or be removed without notice") for
orchestration, kept out of the core CLI. Scheduling is out of scope entirely.

## State tracking and checkpoints

Pi "does not and will not support built-in to-dos" — the author's view is that todo lists
"generally confuse models more than they help" — and it "does not and will not have a built-in
plan mode" ([design blog](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/)). The
endorsed substitute is plain files: `PLAN.md`/`TODO.md` that the agent reads and updates,
which are versioned, diffable, and shareable across sessions.

State tracking instead lives in the **session tree**. Sessions auto-save as JSONL under
`~/.pi/agent/sessions/` organized by cwd; every entry has `id`/`parentId`, forming a tree whose
active leaf is the current position
([sessions.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/sessions.md)).

- `/tree` navigates to any prior node and continues from there *in the same file* (branching), optionally summarizing the abandoned branch into the new position (`BranchSummaryEntry`).
- `/fork` starts a new file from an earlier user message.
- `/clone` duplicates the active branch.

Sessions carry labels, model/thinking changes, compactions, and extension entries; `/export`
writes HTML/JSONL and `/share` uploads a gist. There is **no built-in git
checkpointing/undo** — but the extensions doc lists "Git checkpointing (stash at each turn,
restore on branch)" and "Path protection (block writes to `.env`)" as canonical extension
use-cases, and session branching itself is the primary conversational undo mechanism.

## Self-verification

No built-in lint/test-after-edit loop, no done-ness heuristic, no reflection framework —
consistent with the minimal philosophy. Verification is expected to be driven by the model via
`bash` (run tests, typecheck) and reinforced through context files (`AGENTS.md` telling the
agent which commands to run) or skills that bundle a verification workflow.

The compaction summary's `## Progress → Done/In Progress/Blocked` structure gives a
lightweight surviving record of what's verified vs. pending, but there is no automated
post-edit gate. Extensions could implement one by hooking `tool_result` (after an
`edit`/`write`) or `turn_end` to run tests and feed failures back, but nothing ships out of
the box.

## Ideas worth stealing

1. **Progressive disclosure over up-front tool dumps (anti-MCP).** Pi's central argument: Playwright MCP burns "13.7k tokens (6.8%)" and Chrome DevTools MCP "18.0k tokens (9.0%)" on *every* session, whereas a CLI+README costs ~225 tokens and is pulled in only when needed. BioRouter's built-in MCP servers should audit their always-on tool-description footprint and consider a skills/README pattern for rarely-used tools, loading full schemas on demand.

2. **Structured compaction with a fixed summary schema + never cut a tool pair.** Pi's Goal/Constraints/Progress/Decisions/Next-Steps/Critical-Context template, cumulative file tracking, 2000-char tool-result truncation during summarization, and the hard rule "never cut at tool results" are directly portable and make compaction predictable. BioRouter's `context_mgmt` pruning could adopt the same template and cut-point invariants.

3. **Session-as-tree with in-place branching and branch summaries.** Storing sessions as a `parentId` tree lets users explore alternatives (`/tree`, `/fork`, `/clone`) without spawning files or losing context, and summarizing the abandoned branch injects just its distilled state. This is a lighter, more legible alternative to linear-history-plus-checkpoints for BioRouter's SQLite session store.

4. **A rich, blockable, mutable hook/event lifecycle.** `tool_call` can both block and rewrite arguments in place; `context`/`before_provider_request` can rewrite the outgoing payload; `session_before_compact` can supply a custom summary. Exposing an equivalent typed event bus would let BioRouter ship permission gates, path protection, git checkpointing, and custom compaction as extensions rather than core code.

5. **Trust-gated resource loading, decoupled from runtime sandboxing.** Project Trust cleanly separates "should I load this repo's settings/extensions/skills?" (a solvable input-loading decision) from "is executed code safe?" (which it honestly declares unsolvable in-process). BioRouter's `.biorouterignore`/permission-mode logic could adopt the same explicit split and per-directory `trust.json` memory.

6. **Hot-reloadable, self-authored extensions.** Because extensions are jiti-loaded TypeScript with `/reload`, the agent can write, reload, and test its own extension in a loop. A BioRouter equivalent (agent authors a script/skill, reloads, verifies) would make capability-extension a first-class agent action instead of a human packaging step.

7. **File-based plan/todo instead of a built-in todo tool.** Pi's deliberate refusal of built-in todos/plan-mode in favor of `PLAN.md`/`TODO.md` is worth weighing against BioRouter's approach: plain files are versionable, user-visible, diffable, and survive compaction naturally, avoiding the "todo lists confuse models" failure mode.

## Sources

All web access succeeded; docs were fetched raw from GitHub, so citations are to primary code
and docs.

| Kind | Source |
|---|---|
| Repository | [badlogic/pi-mono](https://github.com/badlogic/pi-mono) (now `earendil-works/pi-mono`) |
| In-repo docs | `packages/coding-agent/docs/` — [usage.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/usage.md), [security.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/security.md), [skills.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/skills.md), [extensions.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/extensions.md), [compaction.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/compaction.md), [sessions.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/sessions.md) |
| In-repo docs | `packages/agent/docs/` — [agent-harness.md](https://github.com/badlogic/pi-mono/blob/main/packages/agent/docs/agent-harness.md) |
| Design blog (2025-11-30) | ["What I learned building an opinionated and minimal coding agent"](https://mariozechner.at/posts/2025-11-30-pi-coding-agent/) |
| Design blog (2025-11-02) | ["What if you don't need MCP at all?"](https://mariozechner.at/posts/2025-11-02-what-if-you-dont-need-mcp/) |
| Independent analysis (2026-01-31) | Armin Ronacher, ["Pi: The Minimal Agent Within OpenClaw"](https://lucumr.pocoo.org/2026/1/31/pi/) |

> **Note.** The design blog posts predate the shipped codebase on compaction, parallel tools
> and session trees; see the provenance callout above. Source paths cite branch `main`, not a
> pinned commit.

## Related documentation

- [OpenCode report](opencode.md) — the same fixed compaction summary schema (Goal / Constraints / Progress / Decisions / Next Steps / Critical Context), arrived at independently.
- [Claude Code report](claude-code.md) — the maximalist opposite of Pi's subtractive thesis, on the same ten dimensions.
- [Aider report](aider.md) — the other deliberately minimal, human-in-the-loop agent in this folder.
- [Session branching design](../../agent-loop/designs/session-branching.md) — BR-45, the BioRouter design closest to Pi's session tree.
- [Improvement proposals register](../../history/agent-loop-review/improvement-proposals.md) — the `BR-NN` index, including BR-9 and BR-11.
