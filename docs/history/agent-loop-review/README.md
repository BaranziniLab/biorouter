# BioRouter agentic loop review

> **What this is.** The executive report of the agentic-loop review: a walkthrough of
> BioRouter's agent feedback loop as it ran at review time, answers to the 14 review
> questions, a comparison against eight open-source coding agents, and the index of the
> 27 sub-reports in this folder.
> **Status:** Historical record — a point-in-time snapshot. The subsystem reviews were
> read against commit `24cdc3a2` on `main` on 2026-07-12. Its findings became the
> `BR-1`…`BR-67` fix campaign, which has since been implemented and merged (86
> `BR-`prefixed commits on `main`), so the loop described below is a **superseded**
> state: gaps reported here as missing — no repo map, no checkpoints, no single-turn
> server lock — have since shipped, and the cited `file.rs:line` positions have drifted.
> Read it as the diagnosis that justified the campaign, not as a description of today's
> code.
> **Audience:** developers working on the agent loop; maintainers.

`BR-NN` identifiers (`BR-1` … `BR-67`) are proposal numbers assigned by this review.
They are defined in full in
[the improvement proposals register](improvement-proposals.md); the campaign that
implemented them cites the same numbers in its commit messages, so they cannot be
renumbered. `P-NN` numbers that appear in the register refer to per-lens proposal ids
inside the three [proposal lenses](proposal-lenses/).

Terms used throughout this report:

- **MOIM** — the per-agent-action `<info-msg>` block injected into the message array,
  carrying the current time, working directory, and each platform extension's live
  state. The acronym is never expanded in the codebase; functionally it is the
  message-of-the-moment ambient-context injection
  ([context injection review](subsystem-reviews/context-injection-and-system-prompt.md)).
- **The tool gauntlet** — the fixed chain of four inspectors (`security`, `permission`,
  `repetition`, `hooks`) every tool request passed through before dispatch
  ([guardrails and permissions review](subsystem-reviews/guardrails-and-permissions.md)).
- **Soft interrupt** — a user message queued mid-turn (via `queue_soft_interrupt`) and
  drained at the top of the next loop iteration, so the user can steer without
  cancelling and resending.
- **Toolshim** — the JSON rewrite applied for models without native function calling
  (`reply_parts.rs:160-166`); at review time it was the only provider-conditional
  transform applied to the prompt.

The review answered a fixed set of questions about how the loop worked, compared each
answer against eight other coding agents, and merged the resulting improvement ideas
into one program. The detailed evidence lives in the ten subsystem reviews, the eight
external tool reports, the four comparison chapters, and the three proposal lenses
listed below.

## Documents in this folder

**Master proposal list**

| Document | What it holds |
|---|---|
| `README.md` (this file) | The executive report and the index. |
| [Improvement proposals register](improvement-proposals.md) | The merged, deduplicated 67-proposal improvement program (`BR-1`…`BR-67`). |

**Proposal lenses** (`proposal-lenses/`) — the three source reviews that were merged
into the register:

| Document | Lens |
|---|---|
| [Performance](proposal-lenses/performance.md) | Latency, token efficiency, caching, startup. |
| [Robustness](proposal-lenses/robustness.md) | Correctness, safety, crash recovery, loop detection. |
| [UX](proposal-lenses/ux.md) | Agent ergonomics, approvals, visibility, control. |

**Comparison chapters** (`competitive-comparison/`) — BioRouter against eight agents,
one chapter per axis:

- [Context injection, system prompts and environment awareness](competitive-comparison/context-and-prompts.md)
- [Compaction and memory](competitive-comparison/compaction-and-memory.md)
- [Safety and guardrails](competitive-comparison/safety-and-guardrails.md)
- [Execution and verification](competitive-comparison/execution-and-verification.md)

**Subsystem reviews** (`subsystem-reviews/`) — BioRouter ground truth, one report per
subsystem:

- [Core loop and tool dispatch](subsystem-reviews/core-loop-and-tool-dispatch.md)
- [Context injection and system prompt](subsystem-reviews/context-injection-and-system-prompt.md)
- [Compaction and context management](subsystem-reviews/compaction-and-context-management.md)
- [Hooks system](subsystem-reviews/hooks-system.md)
- [Guardrails and permissions](subsystem-reviews/guardrails-and-permissions.md)
- [Loop and stuck detection](subsystem-reviews/loop-and-stuck-detection.md)
- [Long-running tasks and scheduling](subsystem-reviews/long-running-tasks-and-scheduling.md)
- [State awareness and version control](subsystem-reviews/state-awareness-and-version-control.md)
- [Server reply flow and session lifecycle](subsystem-reviews/server-reply-flow-and-session-lifecycle.md)
- [Self-verification and done-ness](subsystem-reviews/self-verification-and-doneness.md)

**External tool reports** — moved out of this folder into the coding-agent landscape
research set, since they describe other projects rather than BioRouter:

- [Cline](../../research/coding-agent-landscape/cline.md)
- [OpenCode](../../research/coding-agent-landscape/opencode.md)
- [Pi](../../research/coding-agent-landscape/pi.md)
- [Aider](../../research/coding-agent-landscape/aider.md)
- [OpenHands](../../research/coding-agent-landscape/openhands.md)
- [Codex CLI](../../research/coding-agent-landscape/codex-cli.md)
- [Gemini CLI](../../research/coding-agent-landscape/gemini-cli.md)
- [Claude Code](../../research/coding-agent-landscape/claude-code.md)

`generate_review_html.py` in this folder is the script that rendered the review set as
a single HTML page.

---

## How the loop worked at review time

> **Note.** This section describes commit `24cdc3a2`, before the `BR-1`…`BR-67`
> campaign landed. Several deficiencies it reports were closed by that campaign; see
> [the campaign outcome report](../agent-loop-campaign/outcome-report.md) for what
> actually shipped.

Follow one user message through the pipeline as it ran then.

### From the GUI to the reply route

In the desktop app the message began as an SSE `POST /reply` from the GUI's
`chatStreamStore`, carrying only `session_id` and `user_message` with an
`AbortController.signal`
(see [server reply flow](subsystem-reviews/server-reply-flow-and-session-lifecycle.md)).
The route handler (`reply.rs:214-485`, 50 MB body limit) returned an SSE response
immediately and did all work in a spawned tokio task: it resolved a shared `Arc<Agent>`
from the `AgentManager` LRU, loaded the session's `Conversation` from SQLite, appended
the user message, and called `agent.reply(...)`, which returned a
`BoxStream<AgentEvent>`. A `tokio::select!` loop multiplexed the agent's events, a
500 ms `Ping` heartbeat, and the cancel token, serializing each event as
`data: {json}\n\n`.

Notably there was **no server-side single-turn-per-session lock** — serialization was
only client-side
([server reply flow](subsystem-reviews/server-reply-flow-and-session-lifecycle.md)
gap #1, "the single most important gap").

### Entering `Agent::reply`

`Agent::reply` (`agent.rs:1240`, see
[core loop and tool dispatch](subsystem-reviews/core-loop-and-tool-dispatch.md)) first
handled special cases: an elicitation response short-circuited; `SessionStart` (once)
and `UserPromptSubmit` hooks could block or inject context; slash commands could
resolve the whole reply. It persisted the user message, ran the **auto-compaction
check** (`agent.rs:1432`), then delegated to `reply_internal`.

### Context injection

Context injection happened at three cadences
([context injection review](subsystem-reviews/context-injection-and-system-prompt.md)).
Once at agent construction the system-prompt clock was frozen
(`prompt_manager.rs:186`). Per user turn the entire system prompt was rebuilt from
`prompts/system.md` via MiniJinja — re-reading `.biorouterhints`/`AGENTS.md` from disk
(with `@import` transclusion, boundary/depth/cycle/gitignore-guarded) and each MCP
server's `instructions` (Unicode-sanitized, name-sorted for cache stability). Per agent
action inside the turn, `inject_moim` (`agent.rs:1596`) spliced a fresh `<info-msg>`
user message carrying the time, working directory, and each platform extension's
`get_moim()` — critically the live todo list, which lived in `session.extension_data`
and so survived compaction.

The model was told **one line** about the project ("Working directory: …") and nothing
else — no repo map
([state awareness](subsystem-reviews/state-awareness-and-version-control.md) gap #1).

### The turn loop and the tool gauntlet

The loop (`agent.rs:1556`) then ran turns. Each turn drained soft-interrupts, injected
MOIM, and started the LLM call via `stream_response_from_provider`. Provider SSE deltas
became `MessageStream` items; `categorize_tools` (`agent.rs:1670`) split
frontend/backend tool requests and yielded partial text to the client.

When the model emitted tool requests they ran the tool gauntlet
([guardrails and permissions](subsystem-reviews/guardrails-and-permissions.md)): four
inspectors in fixed order — `security` (a regex scanner, off by default, ask-only),
`permission` (the real gate: `Auto` allowed all, `Approve`/`SmartApprove` routed unknown
tools to human approval — though `SmartApprove` was inert because its read-only sets
were empty and its LLM judge was dead code), `repetition` (`RepetitionInspector`, denied
the 4th byte-identical consecutive call), and `hooks` (user `PreToolUse` hooks). Verdicts
merged **escalation-only** (raise the bar, never lower it).

Approved tools dispatched in parallel via unbounded `select_all` (`agent.rs:1792`); a
`needs_approval` request yielded an `ActionRequired` card and **blocked** on
`confirmation_rx.recv()` until the GUI POSTed `/action-required/tool-confirmation` —
with no TTL, so a lost confirmation blocked forever
([server reply flow](subsystem-reviews/server-reply-flow-and-session-lifecycle.md)
gap #2).

### Tool results, done-ness and persistence

Each tool result was round-trip validated (`call_tool_result::validate`) before
persistence, then `PostToolUse`/`PostToolUseFailure` hooks fired — **observe-only**,
they could not block ([hooks system](subsystem-reviews/hooks-system.md) #2). The
assistant tool-call message and its tool-result message were appended to
`messages_to_add` and yielded over the same SSE stream. Usage was recorded once per turn
(`agent.rs:2035`).

If no tool was called, the loop decided done-ness: auto-continue a length-truncated turn
(dead code on native Anthropic streaming because `finish_reason` was never set —
[core loop](subsystem-reviews/core-loop-and-tool-dispatch.md) gap #1, the "single most
surprising correctness gap"), enforce a pending `final_output` schema, run workflow
retry, or evaluate a Stop hook (the `/goal` judge could block completion).
`messages_to_add` was persisted and merged into the `Conversation` (`agent.rs:2108`).

### Compaction and session persistence

Compaction ([compaction review](subsystem-reviews/compaction-and-context-management.md))
fired proactively at 0.8 of the context limit, reading the provider-reported
`session.total_tokens` (tiktoken `o200k_base` on the cold path). `compact_messages` fed
the *entire* agent-visible history to `complete_fast` (the weak model) producing one
9-section summary; originals were flipped `agent_invisible` but stayed `user_visible`
(non-destructive), and the summary plus a continuation note plus a fresh copy of the last
user message became the new context.

Session persistence was one SQLite DB: a `sessions` row (working_dir, tokens,
`extension_data` JSON) and an append-only `messages` table (`content_json`, positional
synthetic ids). Compaction did a whole-history `DELETE`+re-`INSERT`
([state awareness](subsystem-reviews/state-awareness-and-version-control.md),
[compaction review](subsystem-reviews/compaction-and-context-management.md) gap #8). The
turn ended when the loop broke; the consumer called `maybe_rename_session`.

---

## Answers to the review questions

The 14 answers below record the state at commit `24cdc3a2`.

### When are contexts injected?

At three cadences
([context injection](subsystem-reviews/context-injection-and-system-prompt.md)): once at
agent construction (frozen clock); rebuilt every user turn (system prompt re-rendered,
hints re-read from disk); and per agent action inside a turn (MOIM, soft-interrupts,
truncation nudge). Freshness was best-in-class; the trade was cache churn and a frozen
system-prompt date that contradicted MOIM's clock.

### How do contexts flow through the conversation?

Two independent streams: the system prompt (assembled by `SystemPromptBuilder::build`,
`prompt_manager.rs:104`) and the message array (persisted `Vec<Message>` plus synthetic
per-turn injections). MOIM re-presented durable state (time, cwd, todos) every provider
call and was inserted after trailing tool_results to preserve tool_call/result pairing
([context injection](subsystem-reviews/context-injection-and-system-prompt.md)).

### How does the agent call tools and how do results return?

Function-calling over MCP, name-routed. The model emitted tool requests; inspectors
gated them; approved ones dispatched in parallel; each result was validated, appended as
a tool-response message, and streamed back over SSE
([core loop](subsystem-reviews/core-loop-and-tool-dispatch.md),
[guardrails and permissions](subsystem-reviews/guardrails-and-permissions.md)). Errors
returned as `is_error: true` text the model had to read — there was no structured error
taxonomy.

### How does compaction work and how does the conversation continue after it?

Summarize-everything at 0.8 of the window: the whole agent-visible history → one
`complete_fast` summary; originals went `agent_invisible` (the user still saw them); the
model then saw summary + continuation + re-appended last user message
([compaction review](subsystem-reviews/compaction-and-context-management.md)). No
recent-turn verbatim window — the biggest fidelity gap versus state of the art.

### How do hooks work — how many types and events, where do they inject and enforce?

A Claude-Code-compatible model of shell-command or LLM-judge hooks at **13 wired event
variants** ([hooks system](subsystem-reviews/hooks-system.md)). Four could block
(PreToolUse, PermissionRequest, UserPromptSubmit, Stop); SessionStart/UserPromptSubmit
injected context; PostToolUse was observe-only and PreToolUse `additionalContext` was
silently dropped. Failure-open by design; file/env config only, no rewrite path.

### How are infinite loops and repetitive tool calls handled?

Five layered mechanisms
([loop and stuck detection](subsystem-reviews/loop-and-stuck-detection.md)):
`RepetitionInspector` (byte-exact consecutive duplicates, denying the 4th), a
100-*iteration* turn cap, cooperative cancellation, provider retry/backoff, and
`/goal`-only Jaccard stall detection. Trivially defeated by a one-char arg change or
A/B/A/B oscillation, and the true reason was hidden (the model was told the *user*
declined).

### How are long-running tasks handled?

Four independent systems
([long-running tasks](subsystem-reviews/long-running-tasks-and-scheduling.md)):
well-built background shell jobs (`background=true`, OS-exit-truth, read cursors,
race-free `shell_wait`), blocking semaphore-capped subagents, a durable cron `Scheduler`
with overlap/pause-when-active guards, and elicitation. Background jobs, subagents, and
in-flight scheduled runs did **not** survive a daemon restart.

### How does the agent track processes it started?

Three disjoint in-memory registries with `kill_on_drop(false)` and **no
PID-file/parent-death reaping** — a crash orphaned process groups forever, even though
the llama.cpp sidecar already implemented the reaping pattern; `list()` existed but was
dead-coded so the agent could not enumerate its own jobs
([long-running tasks](subsystem-reviews/long-running-tasks-and-scheduling.md) gaps
#1/#3).

### How does the agent understand its surroundings and track what it is working on?

A single `working_dir` line in MOIM (no repo map, file tree, or symbol index), plus a
full-overwrite `String` todo blob re-injected via MOIM and an in-memory `/goal` state
([state awareness](subsystem-reviews/state-awareness-and-version-control.md) gaps
#1/#3/#4).

### Does it version-control its own edits?

No — no shadow git, no session undo. `git2` was used only for the Knowledge-base wiki;
`text_editor` had an in-memory, per-process, whole-file LIFO that died with the process
and missed shell and other writes
([state awareness](subsystem-reviews/state-awareness-and-version-control.md) gap #2 —
"the single biggest gap").

### How does it know when it is making a mistake?

Almost entirely tool-result `is_error` text; no post-edit compile/lint/LSP loop, no
structured errors, no self-critique. The `/goal` judge was the one active "are you done?"
signal, and loop/stall detection lived only in the goal loop
([self-verification](subsystem-reviews/self-verification-and-doneness.md) gaps #3/#7,
[state awareness](subsystem-reviews/state-awareness-and-version-control.md) gap #7).

### How does it decide explore-versus-answer, and when to harden or test?

Almost entirely prompt tone — no effort/thinking budget, no automatic "this is complex →
plan" trigger; plan mode was a one-shot prompt rewrite. Enforced verification existed
only for workflows (`execute_success_checks`, single `Shell` variant, resetting all
progress on failure); in interactive chat "done" was whatever the model decided
([self-verification](subsystem-reviews/self-verification-and-doneness.md) gaps #1/#9).

### What infrastructure helps it understand the BioRouter ecosystem so it calls tools well?

MOIM injected each platform extension's live state; each MCP server's `instructions`
rendered verbatim under `## <name>`; the OSV malware check gated extension install;
`.biorouterignore` protected secret reads (Developer-MCP only). The gap was a repo map
and per-model prompt variants
([context injection](subsystem-reviews/context-injection-and-system-prompt.md)).

### How does the system prompt teach tool use?

One fixed `system.md` (MiniJinja), with tool schemas passed via function-calling and a
contract-tested set of behavioral clauses; the only per-model transform was the toolshim
JSON rewrite, so 43+ providers shared one prompt regardless of capability
([context injection](subsystem-reviews/context-injection-and-system-prompt.md) gap #10,
[context and prompts comparison](competitive-comparison/context-and-prompts.md)).

---

## How BioRouter compared

### Context and prompts

Per [context and prompts](competitive-comparison/context-and-prompts.md). *Ahead:*
per-turn prompt rebuild with live hint re-read, MOIM durable-state re-injection,
`@import` transclusion with a real security model, and Unicode-tag injection hardening —
best-in-class on freshness and import safety. *Behind:* no repo map (worst-in-class,
alongside Pi — **Aider** does ranked tree-sitter repo-maps best), no per-model prompt
variants (**Codex**), no total context budget (**OpenHands**/**Codex**), and project
files trusted as system-level instruction (**Claude Code** treats them lower-trust).

### Compaction and memory

Per [compaction and memory](competitive-comparison/compaction-and-memory.md). *Ahead:*
non-destructive compaction via visibility flags (the full user transcript survives),
provider-truth-first token accounting, and MOIM keeping task state out of the summarizer.
*Behind:* no recent-turn verbatim window (**Gemini CLI** and **Aider** do this best), no
targeted tool-output offloading (**Claude Code**), an OpenAI-only tokenizer for all
providers (worst-in-class token counting; **Codex** best), and three disjoint,
never-auto-promoted memory stores (**Codex** self-maintained memories best).

### Safety and guardrails

Per [safety and guardrails](competitive-comparison/safety-and-guardrails.md). *Ahead:* a
genuine superset of Claude Code's hook events (13 wired), a fail-closed escalation-only
inspector chain, and unique OSV malware plus local PII guards. *Behind:* no hook
input-rewrite path (**Codex** `updated_input` best), read-only auto-approve and the LLM
judge are dead code (so SmartApprove ≡ Approve; **OpenHands** `security_risk` +
`ConfirmRisky` best), no OS sandbox (**Codex** Seatbelt/Landlock best), an evadable
off-by-default command scanner (**Codex** `execpolicy`), exact-duplicate-only loop
detection (**Gemini CLI** three-layer best), no mistake-streak handling (**Cline** best),
no budget cap, and no checkpoint/undo.

### Execution and verification

Per [execution and verification](competitive-comparison/execution-and-verification.md).
*Ahead:* genuinely well-built background shell jobs (matching **Claude Code**/**Codex**),
fork-bomb-guarded subagents, tool-result validation, soft interrupts, and a
resource-aware scheduler. *Behind:* three conspicuous holes — **no checkpoints/undo**
(worst-in-class, "the single starkest deficit"; **Cline**/**OpenCode** best), **no
automatic post-edit verification** (the LSP/`analyze` capability exists but is unwired;
**Claude Code**/**OpenCode**/**Aider** best), and **no git integration of agent edits**
(**Aider** auto-commit-per-edit best) — plus crude oversized-output handling
(worst-in-class), no per-tool timeouts, unbounded parallel dispatch, and lossy blocking
subagents.

---

## The improvement program

The three lens reviews (performance, robustness, ux) were merged and deduplicated into
**67 proposals (`BR-1` … `BR-67`)** in
[the improvement proposals register](improvement-proposals.md), with every distinct
source proposal preserved (a lens→BR crosswalk verifies coverage).

The two tables below are summary copies of tables that also appear in the register; the
register is authoritative if they ever disagree.

### Counts by category

| Category | Count |
|---|---|
| Context & prompts | 9 |
| Compaction & memory | 8 |
| Hooks & guardrails | 14 |
| Loop & stuck detection | 9 |
| Long-running & processes | 6 |
| Checkpoints & version control | 3 |
| Verification & done-ness | 6 |
| Performance & tokens | 8 |
| UX & agent ergonomics | 4 |
| **Total** | **67** |

### Top 15 by leverage

| id | title | impact | effort | why now |
|---|---|---|---|---|
| BR-46 | Fix Anthropic `finish_reason` → stop silent mid-sentence truncation | High | S | Correctness bug on the *default* provider; every truncated answer ends silently — one-line map, well-scoped test. |
| BR-43 | Shadow-git checkpoints + `/rewind` | High | L | The single starkest deficit vs every current-gen agent; the recovery net that makes aggressive autonomy tolerable. |
| BR-1 | Repo map / workspace summary in context | High | L (M for a file listing) | Worst-in-class per two comparison chapters; the model rediscovers structure every session. |
| BR-18 | Revive read-only auto-approve + risk grading (SmartApprove ≠ Approve) | High | M | Dead code today makes "smart" mode identical to Approve and over-prompts on every read. |
| BR-33 | Single-turn-per-session server lock | High | M | "The single most important gap" in server-flow; raced/duplicate `/reply` corrupts shared state and doubles spend. |
| BR-10 | Recent-turn verbatim window at compaction | High | M | The biggest fidelity regression vs SOTA; freshest tool output/diffs are lost to a lossy summary. |
| BR-52 | Kill per-token DB reads + carry `TokenState` in the SSE event | High | M | #1 streaming-latency source: two SQLite queries per streamed token, one growing with history. |
| BR-6 | Token-aware large-response handling (preview + in-sandbox handle) | High | M | Worst-in-class oversized-output handling; blinds the model on huge grep/SQL/bio outputs. |
| BR-19 | PreToolUse `updated_input` rewrite + let PostToolUse block | High | M | Turns hooks from a veto into a policy engine (sandbox a path, reject a write that fails lint). |
| BR-29 | Staged soft-then-hard loop stop + honest repetition reason | High | S–M | Cheapest loop-safety upgrade; today the model is falsely told the *user* declined. |
| BR-12 | Background compaction off the critical path | High | L | Compaction is a synchronous multi-second LLM stall inside the user's turn. |
| BR-54 | SharedMcpPool across agents/sessions | Very high | L | The dominant RAM multiplier: N sessions × M MCP processes with no sharing. |
| BR-47 | Auto post-edit diagnostics (LSP/`analyze`) feedback loop | High | M | The capability exists but is never wired; turns "run tests" from a suggestion into a signal. |
| BR-20 | Always-on catastrophic-command denylist | High | S–M | `Auto` mode = zero command screening today; a tiny hard-block list closes the hole. |
| BR-30 | Semantic / oscillation / repeated-failing-result loop detection | High | M | Catches the loop classes (arg-tweak, A/B/A/B, repeated errors) that actually occur. |

## Related documentation

- [Improvement proposals register](improvement-proposals.md) — the full `BR-1`…`BR-67` program this report summarizes, with problem, affected code, impact, effort and risk per proposal.
- [Agent-loop fix campaign](../agent-loop-campaign/README.md) — the plan of record for implementing these 67 proposals, including the wave-to-proposal mapping.
- [Agent-loop campaign outcome report](../agent-loop-campaign/outcome-report.md) — what actually landed, so you can tell which gaps above are now closed.
- [Platform parity audit](../agent-loop-campaign/cross-platform/platform-parity-audit.md) — the cross-platform audit that added `BR-68`…`BR-70` after this review.
- [Context engineering guide](../../agent-loop/context-engineering.md) — current documentation of how context reaches the model, superseding this report's snapshot.
