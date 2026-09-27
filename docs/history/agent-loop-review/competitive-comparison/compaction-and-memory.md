# Compaction, memory and session continuity

> **What this is.** Chapter 2 of the four-part competitive comparison in the
> 2026-07 agentic-loop review: how BioRouter's compaction triggers and strategy,
> what survives summarization, token counting, cross-session memory, and session
> persistence/resume compared against nine other open-source coding agents.
> **Status:** Superseded — the BioRouter column no longer describes the system. The
> headline finding "no recent-turn verbatim window" was fixed by BR-10, synchronous
> compaction cost by BR-12, oversized tool output by BR-6, and large-result
> externalization by BR-7 (`crates/biorouter/src/agents/session_blob_tool.rs`). For
> what shipped, read [the agent-loop campaign outcome report](../../agent-loop-campaign/outcome-report.md)
> and [the wave-1 compaction report](../../agent-loop-campaign/wave-reports/wave-1-compaction.md).
> **Audience:** developers working on the agent loop and context management.

This chapter was written on 2026-07-12 as part of the agentic-loop review. It
compares BioRouter against nine open-source coding agents on a single axis: what
happens to conversation history as it grows past the model's context window, and
what the agent remembers between sessions. Read it as a snapshot of the state of
the art at that date and of BioRouter's position in it — not as a description of
current BioRouter behaviour.

Four conventions used throughout:

- **MOIM** is BioRouter's per-action ambient-context block: a fresh `<info-msg>`
  user message carrying the current time, the working directory, and each
  platform extension's contribution, re-injected before every provider call. The
  acronym is not expanded anywhere in the codebase;
  [the context-injection review](../subsystem-reviews/context-injection-and-system-prompt.md)
  treats it as "message of the moment".
- **`BR-NN` identifiers** are proposal numbers from the same review.
  [The improvement proposals register](../improvement-proposals.md) defines
  BR-1…BR-67 with Problem / Proposal / Affected code / Impact / Effort / Risk.
- **Gap citations** take the form *(compaction review, gap #1 — `mod.rs:290-305`)*.
  They name the subsystem review that established the finding, its numbered gap,
  and the source lines that review cited. The two grounding reviews for this
  chapter are [the compaction and context-management review](../subsystem-reviews/compaction-and-context-management.md)
  ("compaction review" below) and [the state-awareness and version-control review](../subsystem-reviews/state-awareness-and-version-control.md)
  ("state-awareness review" below).
- **External claims** are grounded in that tool's report under
  [the coding-agent landscape research](../../../research/coding-agent-landscape/).

The reviewed BioRouter commit was not recorded in the original document.

## Comparison across ten agents

The table has one column per agent, in this order: BioRouter, Goose,
Cline, OpenCode, Pi, Aider, OpenHands, Codex CLI, Gemini CLI, Claude Code. It is
wide and scrolls horizontally.

> **Note.** The BioRouter column is superseded — see the status header. The nine
> competitor columns are a 2026-07 snapshot and have not been re-verified since.

| Aspect | BioRouter | Goose | Cline | OpenCode | Pi | Aider | OpenHands | Codex CLI | Gemini CLI | Claude Code |
|---|---|---|---|---|---|---|---|---|---|---|
| **Auto-compact trigger** | 0.8 of context limit | 0.8 (`GOOSE_AUTO_COMPACT_THRESHOLD`) | ratio of resolved `maxInputTokens` | overflow = window − reserve(32k) − buffer(20k) | `contextTokens > window − reserve(16384)` | `too_big()` vs `max_tokens` budget | `EVENTS`>80 msgs / `TOKENS` / `REQUEST` | auto when context fills + mid/pre-turn | 0.5 of window | ~83.5% of window |
| **Reactive on overflow** | Yes, capped 2 attempts | Yes (`GOOSE_CONTEXT_STRATEGY` ladder) | rule-based fallback | `isOverflow` hard path | auto-compact + retry aborted turn | breaks, warns `/drop` | `CondensationRequest` + 5× hard-reset | escalation via lifecycle | inflation-guard discard | offloading + compact |
| **Recent-turn verbatim window** | No — summarizes everything | recent user msgs kept | `preserveRecentTokens` (agentic) | protects last 2 turns | `keepRecentTokens` (20k) | tail kept verbatim (~half budget) | recent tail + `keep_first=4` head | handoff keeps progress | last 30% verbatim | keeps recent, drops verbatim |
| **Summary structure** | 9-section schema (agent-facing) | `compaction.md` template | incremental note + Files section | Goal/Constraints/Progress/… | fixed Goal/…/Critical-Context | weak-model prose summary | structured, preserves task IDs | handoff schema | `<state_snapshot>` + verify pass | named schema (intent/files/errors/…) |
| **Targeted tool-output pruning** | Only during summarizer overflow (whole responses, middle-out) | Background `GOOSE_TOOL_CALL_CUTOFF=10` | basic-compaction truncates tool results | prune-first (>40k, protect last 2, skip skills) | truncate tool results to 2000 chars | no (re-injects files fresh) | per-event 0.8× shrink | `TruncationPolicy` | reverse-budget: old >50k → 30 lines | Bash output → file + head preview |
| **Single oversized message** | Hard fail → "start new session" | truncate/clear fallback | rule-based trim | media→placeholder | "split turn" mid-turn cut | suggests `/drop`,`/clear` | `hard_context_reset` shrink | `new_context_window` | truncate old responses | offload to file |
| **Token counting** | tiktoken `o200k_base`, one encoding all providers | context_mgmt module | per-model `maxInputTokens` | tokens vs usable window | `contextTokens` vs window | tokenizer vs `max_tokens` | condenser `max_tokens` | model-callable `get_context_remaining` | streamed usage | window-usage % |
| **Provider-truth first** | Yes (`session.total_tokens`), tiktoken cold path | provider usage | model-info limits | provider usage | provider usage | model input limit | provider usage | provider + server-side | provider usage | provider usage |
| **Compaction model** | fast/weak (`complete_fast`) | summarization LLM | agentic (same) / rule fallback | summary agent + "no-LLM" memory path | main or cheaper via hook | weak model | summarization LLM | model or remote | LLM + verify pass | main model |
| **Cross-session memory** | 3 disjoint: chatrecall (LIKE), Knowledge KB (git wiki, opt-in), ingest | Memory MCP (tagged, agent-writable) | Memory Bank (`.clinerules` convention) | AGENTS.md | files (`TODO.md`) + session tree | git history + `CONVENTIONS.md` | event store + AGENTS.md | `~/.codex/memories/` ranked, self-maintained | GEMINI.md + Auto Memory inbox | CLAUDE.md + Auto memory (`MEMORY.md`) |
| **Auto-promoted memory** | No (Soul opt-in per query) | on-demand by tag | manual | no | no | no | no | Yes (consolidation sub-agent) | Yes (idle-mining → review inbox) | Yes (first 200 lines auto-load) |
| **Session store** | SQLite, append-only messages + `extension_data` JSON | data-dir JSON | disk sessions + snapshots | SQLite (Drizzle) | JSONL session tree | `.aider.chat.history.md` | persisted event store | rollout files `~/.codex/` | session JSON | session store |
| **Resume / branching** | Load full history each turn; `diverged_from` | resume | versioning/snapshots | SSE replay | `/tree` `/fork` `/clone` branching | restore prior msgs | replay + fork | resume + replay | rewind across compression | rewind + worktrees |
| **Compaction hooks** | Pre/PostCompact | Pre/PostCompact | `PreCompact` | `session.compacted` | `session_before_compact` (override) | none | condenser pluggable | Pre/Post-Compact (can abort) | `PreCompress` | Pre/PostCompact (can block) |

## Where BioRouter was ahead

- **Non-destructive compaction via visibility flags.** BioRouter never deletes
  history: compaction flips originals to `agent_invisible` while keeping them
  `user_visible`, so the user's transcript survives intact even though the model
  only sees summary + continuation + the re-appended last user message
  (compaction review — `message.rs:509-577`, `mod.rs:119-164`). Claude Code and
  Gemini CLI, by contrast, *replace* the verbatim conversation — full tool outputs
  and intermediate reasoning are simply gone. BioRouter's clean separation of
  "what the model sees" from "what the user sees" is a genuinely good primitive
  most competitors lack.
- **Provider-truth-first token accounting.** The 0.8 check reads the provider's
  reported `session.total_tokens` and only falls back to tiktoken on a cold turn
  (compaction review — `mod.rs:184-213`). Several tools estimate locally; using
  the billed ground truth and offloading BPE to `spawn_blocking` is the right call.
- **MOIM re-injection keeps durable task state out of the summarizer.** Todos
  live in `extension_data` (SQLite), not the message log, and are re-injected
  every provider call, so they survive compaction verbatim rather than depending
  on the summary capturing them (state-awareness review — `moim.rs:12`,
  `todo_extension.rs:197`). This is structurally better than Cline's and Gemini's
  in-history todo lists that must be re-summarized to survive.
- **Correct tool_call/tool_result pairing across the boundary.** Because
  compaction fully replaces the agent-visible slice and `fix_conversation`
  cleans orphaned pairs, no dangling tool messages cross the boundary
  (compaction review — `conversation/mod.rs:307-399`). Pi and Cline had to add
  explicit "never cut a tool pair" invariants; BioRouter gets it for free.
- **Three-tier trigger with observable hooks.** Proactive-at-0.8, reactive-on-
  overflow, and manual `/compact`, each firing Pre/PostCompact hooks — good
  coverage, on par with Codex and Claude Code and ahead of Aider (no hooks at all).

## Where BioRouter was behind

> **Warning.** Several findings below were fixed after this review — see the
> status header. They are preserved as the record of what the review found.

- **No recent-turn verbatim window (biggest fidelity gap).** BioRouter collapses
  the *entire* agent-visible history — including the latest tool outputs, diffs,
  and errors — into one lossy summary; only the last plain-text user message
  survives verbatim (compaction review, gap #1 — `mod.rs:290-305`,
  `mod.rs:155-159`). **Gemini CLI does this best and is directly
  reimplementable:** `COMPRESSION_PRESERVE_THRESHOLD = 0.3` keeps the last 30% of
  history verbatim and summarizes only the older 70%; `findCompressSplitPoint()`
  snaps the boundary to the most recent user message with no function responses (a
  clean turn). Aider is equally concrete: split head/tail backward accumulating to
  ~half the budget, snap so the head ends on an assistant message, summarize only
  the head with the weak model, recurse if still too big (depth>3 →
  summarize_all). OpenHands pins a `keep_first=4` head and keeps a `max_size//2`
  tail.
- **No targeted tool-output pruning/offloading.** A single giant grep/SQL/
  bioinformatics result poisons the window and can only be removed *whole*, and
  only when the summarizer itself overflows (compaction review — `mod.rs:236-284`).
  **Claude Code's offloading is the cleanest to copy:** cap Bash output at ~30 KB,
  spill the rest to a file in the session dir, and hand the model the path + a
  head preview so it greps on demand — lossless, no summarization. **OpenCode's
  prune-first** is the best compaction-layer variant: scan newest→oldest, protect
  the last 2 turns, prune tool bodies only when total tool output >40k and ≥20k is
  reclaimable, never prune `skill` outputs. **Gemini's reverse-budget** truncates
  old tool responses >50k tokens down to 30 lines before summarizing. **Goose**
  already added background summarization of tool pairs older than
  `GOOSE_TOOL_CALL_CUTOFF=10`, a gap compared with Goose that BioRouter has not
  closed.
- **Individual over-window message is a dead end.** No head/tail truncation of a
  single oversized payload; BioRouter tells the user to start a new session
  (compaction review, gap #3 — `agent.rs:1967-1975`). Pi's "split turn" (cut
  mid-turn, emit two merged summaries) and OpenHands' `hard_context_reset` (retry
  ≤5×, shrink per-event strings 0.8× each) both degrade gracefully instead.
- **Summarizer is the *fast/weak* model.** `do_compact` calls `complete_fast`, so
  the cheapest model writes the memory the strong model then relies on — and its
  smaller window overflows more often (compaction review, gap #2). Claude Code
  summarizes with the main model; Gemini adds a second verification pass; OpenCode
  and Codex allow a server-side/no-LLM path. No summary validation, and the
  summary's role is force-set to `Role::User` (a mislabel).
- **Single OpenAI tokenizer for all providers + cold-path undercount.**
  `o200k_base` is wrong for Claude/Gemini/Bedrock/Ollama, and the fallback
  estimate uses `count_chat_tokens("", &[msg], &[])` — no system prompt, no tool
  schemas, the two largest contributors (compaction review, gaps #4–#5). Codex's
  model-callable `get_context_remaining`/`new_context_window` is the mature
  answer; at minimum the cold path should include system + tools.
- **Cross-session memory is three disjoint stores, none auto-promoted.**
  Chatrecall is substring `LIKE` OR-match ranked only by recency; Knowledge bases
  and conversation-ingest are separate; "Soul" is opt-in per query, never
  auto-injected (state-awareness review, gaps #5–#6). **Codex CLI does memory
  best:** `~/.codex/memories/` distills finished sessions into ranked, cited
  memories (`usage_count`/`last_usage`) injected as developer instructions, with a
  Phase-2 consolidation sub-agent. **Claude Code's Auto memory** auto-loads the
  first 200 lines/25 KB of `MEMORY.md` into every session; **Gemini's Auto
  Memory** mines idle transcripts into a human-approved review inbox. All three
  beat BioRouter's opt-in, unindexed, substring-search model.
- **No structured task-preservation in the summary.** OpenHands' condenser prompt
  forces a `TASK_TRACKING` section that preserves exact task IDs and statuses;
  Cline re-attaches a Files section. BioRouter leans on MOIM re-injection instead,
  which works for todos but not for arbitrary plan/file state captured only in
  history.
- **Whole-history rewrite every compaction.** `replace_conversation_inner` does
  `DELETE`+re-`INSERT` of the entire (ever-growing, mostly agent-invisible)
  message set, and `get_conversation` deserializes all of it every turn
  (compaction review, gap #8) — O(n) I/O with no archival. OpenCode's
  SQLite/Drizzle and OpenHands' append-only replayable event store scale better.

## Best-in-class and worst-in-class per aspect

- **Compaction strategy (what survives):** *Best — Gemini CLI* (30% verbatim tail
  + structured `<state_snapshot>` + self-correcting verification pass + inflation
  guard that discards a summary if it grew tokens). Runners-up: OpenCode
  (prune-before-summarize, skip-LLM memory path) and OpenHands (pinned head,
  minimum-progress guard, graceful hard-reset). *Worst — BioRouter and Codex's
  token-budget mode* both throw away recent verbatim context; but BioRouter is
  worse because it *also* uses the weak model and has no verbatim window at all.
  Reasoning: keeping recent turns verbatim is the single highest-fidelity, lowest-
  risk choice, and BioRouter is the only agent here that keeps essentially none.
- **Targeted tool-output handling:** *Best — Claude Code* (offload-to-file with
  head preview: lossless and O(1)). *Worst — Aider* (no pruning; but it sidesteps
  the problem by re-injecting files fresh each turn) and *BioRouter* (only whole-
  response removal, only on summarizer overflow). BioRouter is genuinely worst
  among agents that keep tool output in history.
- **Token counting accuracy:** *Best — Codex CLI* (model-callable budget +
  server-side awareness). *Worst — BioRouter* (one OpenAI tokenizer for every
  provider, plus a cold-path estimate that omits system prompt and tools). This
  is unambiguous: BioRouter's approximation error is structurally larger than any
  competitor's.
- **Cross-session memory:** *Best — Codex CLI* (self-maintained, ranked, cited
  memories) with Claude Code and Gemini close behind (auto-load + review inbox).
  *Worst — Aider* (deliberately none beyond git + CONVENTIONS.md) and *Pi* (files
  only, by philosophy). BioRouter sits mid-pack: it *has* rich stores (git-backed
  KB) but they are disjoint, unindexed for chat, and never auto-promoted, so
  effective recall is weak.
- **Session persistence & non-destructive resume:** *Best — BioRouter* actually
  wins on non-destructiveness (visibility flags preserve the full user transcript
  through compaction) — tied with OpenHands' replayable event store and OpenCode's
  SSE-replay. *Worst — Aider* (flat markdown history, no structured store). Pi's
  session-as-tree (`parentId`, `/tree`/`/fork`/`/clone`) is the best *branching*
  model; BioRouter has only `diverged_from` and renumbers positional message ids
  on rewrite (state-awareness review, gap #10).
- **Summary model choice:** *Best — Claude Code* (main model). *Worst — BioRouter
  and Aider* (weak model writes the memory), though Aider mitigates by preserving
  the verbatim tail so the weak summary only covers the older head.

## Implications and where they landed

The six items below were the chapter's own recommendations. All of them were
consolidated into [the improvement proposals register](../improvement-proposals.md),
which is the authoritative list — read this section as the argument behind the
proposals, not as an open work queue. The register's BR-NN number is noted where
the mapping is one-to-one.

1. **Add a recent-verbatim window — the top priority** (became **BR-10**). Adopt
   Gemini/Aider's split: keep the last ~30% (or a `keepRecentTokens` budget)
   verbatim, summarize only the older prefix, and snap the cut to a clean user
   turn that has no pending tool responses. This is the single biggest fidelity
   regression vs the state of the art and directly reuses BioRouter's existing visibility-flag
   machinery.

2. **Offload large tool outputs instead of summarizing them** (became **BR-6**,
   with externalization of large results as **BR-7**). Copy Claude Code's pattern
   in the shell/`text_editor` tools: cap output, spill the remainder to a session
   file, return path + head preview. Complement with OpenCode-style
   prune-before-summarize (protect last N turns, prune only bodies above a token
   floor, never prune skill/critical outputs). Goose's background tool pair
   summarization is a ready design to adopt.

3. **Fix token accounting.** At minimum, include the system prompt and tool
   schemas in the cold-path estimate; ideally add per-provider tokenizers (or a
   model-callable budget like Codex). The current single-`o200k_base`,
   no-system/no-tools estimate makes the 0.8 threshold an uncalibrated guess for
   every non-OpenAI model.

4. **Harden the summarizer.** Use the main (or a configurable) model rather than
   `complete_fast`, add empty/length/format validation with one retry, stop
   forcing the summary to `Role::User`, and adopt a structured schema that
   explicitly preserves task IDs/plan/file state (OpenHands `TASK_TRACKING`,
   Cline Files section) so plan continuity does not depend solely on MOIM.

5. **Graceful degradation for a single oversized message** (Pi split-turn /
   OpenHands hard-reset) instead of the current "start a new session" dead end.

6. **Unify and auto-promote cross-session memory.** Index chat with SQLite FTS5
   (already available) rather than substring `LIKE`, and add Codex/Claude-style
   auto-distillation of finished sessions into a ranked, auto-injected memory
   file — closing the gap between BioRouter's strong-but-disjoint stores and the
   competitors' single always-consulted memory.

## Related documentation

- [Compaction and context management](../subsystem-reviews/compaction-and-context-management.md) — the internal review this chapter's BioRouter compaction claims are drawn from, with the numbered gaps cited above.
- [State awareness and version control](../subsystem-reviews/state-awareness-and-version-control.md) — the internal review behind the cross-session memory and session-persistence claims.
- [Context injection, system prompts and environment awareness](context-and-prompts.md) — the sibling chapter covering what goes *into* the window, where this one covers what happens when it fills; the two overlap on MOIM and durable memory.
- [The improvement proposals register](../improvement-proposals.md) — BR-1…BR-67, the consolidated list this chapter's implications fed into.
- [Wave-1 compaction report](../../agent-loop-campaign/wave-reports/wave-1-compaction.md) — what actually shipped against these findings, and therefore the current truth.
