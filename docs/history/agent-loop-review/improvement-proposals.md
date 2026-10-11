# Agentic loop improvement proposals

> **What this is.** The merged, deduplicated master register of the 67 improvement
> proposals (`BR-1` … `BR-67`) raised by the agentic-loop review, each with problem,
> proposal, inspiration, affected code, impact, effort and risk.
> **Status:** Historical record — this register specified a campaign that has since
> been executed. All 67 proposals were implemented and merged to `main` (86
> `BR-`prefixed commits), including every item in the Top 15 by leverage. There is no
> per-proposal status column here, so this is **not** a live backlog; for what
> actually shipped per proposal, read
> [the campaign commit log](../agent-loop-campaign/commit-log.md) and
> [the outcome report](../agent-loop-campaign/outcome-report.md). The `file.rs:line`
> citations describe commit `24cdc3a2` and have drifted since.
> **Audience:** developers working on the agent loop; maintainers.

Three identifier schemes are live in this file, and all three are cited elsewhere, so
none can be renumbered:

| Scheme | Meaning | Where it is defined |
|---|---|---|
| `BR-1` … `BR-67` | A proposal in this register. Campaign commit messages carry a `BR-NN:` prefix. | This file. `BR-68`…`BR-70` were added later by [the platform parity audit](../agent-loop-campaign/cross-platform/platform-parity-audit.md). |
| `P` / `R` / `U` | The **Lens** tag on each entry: which of the three source reviews raised it — P = performance, R = robustness, U = ux. | [proposal-lenses/](proposal-lenses/) |
| `performance P-3`, `ux P-13`, … | A proposal id *inside* one lens file. The lens name always precedes the number, because each lens numbers from `P-1` independently. | [The lens-to-BR crosswalk](#appendix-lens-to-br-crosswalk) maps every one to its `BR-NN`. |

Every distinct improvement proposal surfaced by the three lens reviews
([performance](proposal-lenses/performance.md),
[robustness](proposal-lenses/robustness.md), [ux](proposal-lenses/ux.md)) appears here,
grounded in the ten [subsystem reviews](subsystem-reviews/) and the four
[comparison chapters](competitive-comparison/). Where the same idea appeared in more
than one lens, the richer writeup was kept and the lens tags note the overlap. The
executive report that frames this program is
[the review README](README.md), which also carries summary copies of the two tables
below; this register is authoritative if they disagree.

Each entry carries **Problem / Proposal / Inspired by / Affected code / Impact / Effort
/ Risk**, on these scales:

- **Impact:** Low / Medium / High / Very high. A range (`Medium–high`) means the entry
  spans two levels depending on deployment.
- **Effort:** S (hours) / M (days) / L (weeks). A range (`S–M`, `M–L`) means the same.
- **Inspired by:** the agent whose design the proposal borrows. The value **`novel`**
  means no external agent supplied the pattern — the proposal originated in this
  review; a parenthetical after it names the concern (for example
  `novel (dead-code hygiene)`).

## Contents

| Category | IDs | Count |
|---|---|---|
| [Context & prompts](#context--prompts) | BR-1 … BR-9 | 9 |
| [Compaction & memory](#compaction--memory) | BR-10 … BR-17 | 8 |
| [Hooks & guardrails](#hooks--guardrails) | BR-18 … BR-28, plus BR-64, BR-65, BR-67 | 14 |
| [Loop & stuck detection](#loop--stuck-detection) | BR-29 … BR-36, plus BR-66 | 9 |
| [Long-running & processes](#long-running--processes) | BR-37 … BR-42 | 6 |
| [Checkpoints & version control](#checkpoints--version-control) | BR-43 … BR-45 | 3 |
| [Verification & done-ness](#verification--done-ness) | BR-46 … BR-51 | 6 |
| [Performance & tokens](#performance--tokens) | BR-52 … BR-59 | 8 |
| [UX & agent ergonomics](#ux--agent-ergonomics) | BR-60 … BR-63 | 4 |
| **Total** | | **67** |

> **Why two category ranges are discontinuous.** `BR-64` … `BR-67` were originally
> folded into broader clusters and were promoted late to standalone proposals (OS
> sandbox, managed policy tier, mistake-streak counter, loop-safety observability) so
> that no distinct source proposal was bundled away. They therefore count towards
> Hooks & guardrails and Loop & stuck detection but carry ids past the end of those
> ranges, and their full writeups are collected together under
> [Promoted standalone proposals](#promoted-standalone-proposals) rather than inside
> their category sections.

## Top 15 by leverage

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

---

## Context & prompts

### BR-1: Repo map / workspace summary in context
- **Lens:** U (ux P-13); reinforced by `competitive-comparison/context-and-prompts.md`, `competitive-comparison/execution-and-verification.md`.
- **Problem:** The model is told **one line** ("Working directory: …", `extension_manager.rs:1490`) and nothing else — no file tree, no symbol map, no `ls` snapshot (`subsystem-reviews/state-awareness-and-version-control.md` gap #1). `competitive-comparison/context-and-prompts.md` calls this BioRouter's single largest gap and worst-in-class; the agent clumsily rediscovers structure every session.
- **Proposal:** Inject a cached, gitignore-aware workspace summary into MOIM or the system prompt. Start with a cwd file listing (Cline/Claude-Code level); graduate to an Aider-style ranked tree-sitter repo-map with a token budget.
- **Inspired by:** Aider (ranked repo-map), Cline (`environment_details`), Claude Code (dir snapshot).
- **Affected code:** new module feeding `extension_manager.rs:1490` MOIM / `prompt_manager.rs`; reuse the `analyze` tree-sitter machinery.
- **Impact:** High — biggest single competence/UX gap per two reviews.
- **Effort:** L (M for a plain file listing).
- **Risk:** Cost/staleness; must respect `.biorouterignore` and cache invalidation.

### BR-2: Introduce a total context budget with ranking/truncation
- **Lens:** P (performance P-3); reinforced by `competitive-comparison/context-and-prompts.md`.
- **Problem:** No aggregate size/token accounting anywhere. Hints, extension instructions, inlined skill bodies, and MOIM are concatenated with only a 128 KB per-file *parse* cap (`import_files.rs:53`; `subsystem-reviews/context-injection-and-system-prompt.md` gap #1). A large `AGENTS.md`, chatty MCP server, or several `@import`s can silently blow the window.
- **Proposal:** Add a token-budgeted assembler that measures each injected block, ranks by relevance/recency, and truncates or drops lowest-priority blocks to fit a configurable budget. Mirror OpenHands' pinned-head + minimum-progress condenser and Codex's `project_doc_max_bytes` cap.
- **Inspired by:** OpenHands (`LLMSummarizingCondenser`), Codex CLI (`project_doc_max_bytes`).
- **Affected code:** `crates/biorouter/src/agents/prompt_manager.rs`, `hints/load_hints.rs`, `agents/moim.rs`.
- **Impact:** High — bounds worst-case token spend and prevents silent overflow-induced compaction thrash.
- **Effort:** M.
- **Risk:** Over-aggressive truncation could drop guidance the model needed.

### BR-3: Per-model system-prompt variants
- **Lens:** P + U (performance P-38, ux P-20); reinforced by `competitive-comparison/context-and-prompts.md`.
- **Problem:** One fixed `system.md` serves 43+ providers of wildly varying capability; the only per-model transform is the toolshim JSON rewrite (`reply_parts.rs:160-166`) (`subsystem-reviews/context-injection-and-system-prompt.md` gap #10). Strong models get verbose guidance they don't need (wasted tokens); weak/local models (Llama Server, Ollama) get too little scaffolding, and ask-vs-act / verification rigor cannot be tuned to model strength.
- **Proposal:** A provider/model-keyed prompt-variant table with a default fallback (Codex pattern), a shared base + overrides to avoid sprawl, and a retained contract test per variant.
- **Inspired by:** Codex CLI (per-model prompt files, e.g. `gpt-5.2-codex_prompt.md`).
- **Affected code:** `crates/biorouter/src/agents/prompt_manager.rs`, `prompts/system.md` → variant registry, `reply_parts.rs` prompt selection.
- **Impact:** Medium — token savings on strong models + better behavior on weak ones.
- **Effort:** M–L.
- **Risk:** Variant sprawl / maintenance burden; keep variants minimal.

### BR-4: Move core disciplines into `system.md`
- **Lens:** U (ux P-22); reinforced by `competitive-comparison/context-and-prompts.md` implication #6.
- **Problem:** Planning/todo and tool-batching guidance live **only** in the todo/code-execution extensions' `get_moim`, so a session without those extensions loses the guidance entirely.
- **Proposal:** Base the core planning/batching/verification norms in `system.md` so they hold regardless of which extensions are enabled; keep extension MOIM for live *state*, not behavioral *rules*.
- **Inspired by:** Codex / Claude Code (disciplines in the base prompt).
- **Affected code:** `crates/biorouter/src/prompts/system.md`; `todo_extension.rs` / code-execution `get_moim`.
- **Impact:** Medium — consistent behavior across configs.
- **Effort:** S.
- **Risk:** Prompt bloat; keep it tight.

### BR-5: Dedup MOIM and refresh the system-prompt clock
- **Lens:** P + U (performance P-4, P-5, ux P-30).
- **Problem:** `inject_moim` runs every loop iteration (`agent.rs:1596`) with no removal of the prior block, so a long multi-tool turn accumulates repeated near-identical `<info-msg>` blocks (`subsystem-reviews/context-injection-and-system-prompt.md` gap #3). Separately, `current_date_timestamp` is frozen at agent construction (`agent.rs:253`, `prompt_manager.rs:186`) at UTC-hour granularity while MOIM uses Local-minute — the model sees two contradictory clocks (`subsystem-reviews/context-injection-and-system-prompt.md` gap #2).
- **Proposal:** Before inserting a fresh MOIM, strip the previous agent-only MOIM block so exactly one is present. Recompute the date per turn at hour granularity (cache-stable within the hour), or drop the system-prompt date and rely on MOIM's timestamp — removing the contradiction and a source of cache churn.
- **Inspired by:** Codex CLI (`current_time.rs` per-turn context fragments); novel (dedup).
- **Affected code:** `crates/biorouter/src/agents/moim.rs`, `prompt_manager.rs`, `agent.rs:253`, `reply_parts.rs`.
- **Impact:** Medium — cuts redundant tokens per multi-tool turn and removes contradictory timestamps.
- **Effort:** S.
- **Risk:** Must not remove a MOIM already merged into a user turn by `fix_conversation`; per-second refresh would bust the cache (keep hour granularity).

### BR-6: Token-aware large-response handling (head/tail preview + in-sandbox handle)
- **Lens:** P + R + U (performance P-7, robustness P-42, ux P-21); worst-in-class per `competitive-comparison/execution-and-verification.md`.
- **Problem:** `process_tool_response` uses a 200,000-**character** (~50k token) threshold applied per content item, so several items just under threshold still blow the context; the remediation dumps to `std::env::temp_dir()` outside the session working dir with **no preview, no line count, no token-aware truncation** (`subsystem-reviews/core-loop-and-tool-dispatch.md` gap #4, `large_response_handler.rs`).
- **Proposal:** Switch to a token-aware, aggregate threshold; inline a bounded head/tail preview plus a line-count summary and a searchable handle that resolves inside the session working dir. Add head/tail middle-out truncation ("…N tokens elided…") for an individual over-window message.
- **Inspired by:** Claude Code / Gemini CLI (cap → file + head preview), OpenCode (prune-before-summarize).
- **Affected code:** `crates/biorouter/src/agents/large_response_handler.rs`.
- **Impact:** High — directly reduces tokens from oversized tool results, the biggest single-turn blowup source; keeps the model working on huge grep/SQL/bioinformatics outputs.
- **Effort:** M.
- **Risk:** Truncating a result the model needed in full; mitigate with the searchable handle and head+tail (not head-only).

### BR-7: Externalize large tool results from `content_json`
- **Lens:** P (performance P-8).
- **Problem:** Large tool responses are serialized whole into `messages.content_json` (`session_manager.rs:1857`); message load deserializes all of it eagerly, bloating session DBs and slowing every load (`subsystem-reviews/state-awareness-and-version-control.md` gap #9).
- **Proposal:** Store payloads over a threshold in a side blob table (or file) referenced by a handle; load lazily only when the model requests them.
- **Inspired by:** novel.
- **Affected code:** `crates/biorouter/src/session/session_manager.rs`, `agents/large_response_handler.rs`.
- **Impact:** Medium — faster session load, smaller DBs, less RAM on `get_conversation`.
- **Effort:** L.
- **Risk:** Migration + backward compatibility for existing sessions.

### BR-8: Cap and cache eager skill-body inlining
- **Lens:** P (performance P-6).
- **Problem:** `skill_resource_context` calls `loadSkill` and inlines the entire skill body into a synthetic user message every turn the ref appears, with no size cap and no caching across turns (`agent.rs:506-535`; `subsystem-reviews/context-injection-and-system-prompt.md` gap #7).
- **Proposal:** Cache the loaded skill body per session, inject it once, and cap its size against the BR-2 context budget; on repeat turns inject a short pointer rather than the full body.
- **Inspired by:** novel.
- **Affected code:** `crates/biorouter/src/agents/agent.rs:506-535`, `resource_refs.rs`.
- **Impact:** Medium — removes repeated multi-KB injections on skill-heavy sessions.
- **Effort:** M.
- **Risk:** A skill body edited mid-session would be stale until cache invalidation.

### BR-9: Treat project context files as lower-trust
- **Lens:** (`competitive-comparison/context-and-prompts.md` implication #3 — cross-cutting, surfaced in the comparison layer).
- **Problem:** BioRouter inserts hint bodies and their `@import` expansions verbatim (Unicode-tag-stripped only) with no "treat as untrusted data" framing (`subsystem-reviews/context-injection-and-system-prompt.md` gap #5), so a malicious repo `AGENTS.md` becomes system-prompt-level instruction. Worst-in-class for trust, tied with Pi.
- **Proposal:** Wrap hint/`@import` bodies in explicit data-not-instruction framing and/or gate loading behind a per-directory trust decision (Pi's Project Trust `ask/always/never`). Keep the existing Unicode-tag sanitization; extend the trust boundary to content semantics, not just character classes.
- **Inspired by:** Claude Code (lower-trust project files), Pi (`~/.pi/agent/trust.json`).
- **Affected code:** `crates/biorouter/src/agents/prompt_manager.rs`, `hints/load_hints.rs`, `import_files.rs`.
- **Impact:** Medium–high — closes a system-prompt-injection surface for one-click-installed repos.
- **Effort:** M.
- **Risk:** Over-framing could make the model ignore legitimate project guidance.

---

## Compaction & memory

### BR-10: Keep a recent-turn verbatim window at compaction
- **Lens:** R (robustness P-41); top priority in `competitive-comparison/compaction-and-memory.md`.
- **Problem:** Compaction is summarize-everything: the *entire* agent-visible history — including the most recent tool outputs, diffs, file contents, and errors — is collapsed into lossy prose; only one plain-text user message survives verbatim (`subsystem-reviews/compaction-and-context-management.md` gap #1 — "the single biggest fidelity regression vs SOTA").
- **Proposal:** Keep the last N turns verbatim (or a token-budgeted recent window) and summarize only the older prefix, snapping the cut to a clean user turn with no pending tool responses. Reuses BioRouter's existing visibility-flag machinery.
- **Inspired by:** Gemini CLI (`COMPRESSION_PRESERVE_THRESHOLD = 0.3` + `findCompressSplitPoint`), Aider (head/tail split), OpenHands condenser (`keep_first=4`).
- **Affected code:** `crates/biorouter/src/context_mgmt/mod.rs:50-164,286-349`.
- **Impact:** High — major fidelity improvement across the compaction boundary.
- **Effort:** M.
- **Risk:** Must keep tool_call/result pairs intact within the kept window.

### BR-11: Head/tail-truncate a single over-window message (remove the "start over" cliff)
- **Lens:** R (robustness P-42, overlaps BR-6); `competitive-comparison/compaction-and-memory.md`.
- **Problem:** A single message that alone exceeds the window (e.g. a 400k-token tool result or user paste) is a hard dead end — after removing all *whole* tool responses, `do_compact` errors and the loop tells the user to start a new session (`subsystem-reviews/compaction-and-context-management.md` gap #3, `subsystem-reviews/core-loop-and-tool-dispatch.md` gaps #4/#6, `mod.rs:336-338`, `agent.rs:1967-1975`).
- **Proposal:** Add head/tail middle-out truncation for a single oversized message and/or a token-aware clamp on tool results before they enter history (complements the 200k-char handler in BR-6).
- **Inspired by:** Pi (split-turn), OpenHands (`hard_context_reset`).
- **Affected code:** `crates/biorouter/src/context_mgmt/mod.rs:236-284`, `agents/large_response_handler.rs`.
- **Impact:** High — removes a "start over" cliff.
- **Effort:** M.
- **Risk:** Truncation can drop the important part — prefer head+tail.

### BR-12: Move auto-compaction off the user-visible critical path
- **Lens:** P (performance P-9).
- **Problem:** Compaction is a synchronous LLM round-trip inside `reply()` — the user waits for a summarization call, and `do_compact` can retry up to 5× with progressive tool-response removal, all blocking (`../performance-2026-06/jcode-comparison-analysis.md:157-163`).
- **Proposal:** Trigger background compaction at ~80% budget via `tokio::spawn`, swap the compacted history in on a later turn; keep only a synchronous no-LLM hard-drop at ~95% as a floor.
- **Inspired by:** jcode (`compaction.rs:860-1027`, background compaction + 0.95 hard-drop).
- **Affected code:** `crates/biorouter/src/context_mgmt/mod.rs`, `agents/agent.rs:1432,1478`.
- **Impact:** High — removes a multi-second stall from the user's turn.
- **Effort:** L.
- **Risk:** Racing a background swap against a live turn needs careful history-version handling.

### BR-13: Progressive context-overflow fallback instead of a 2-attempt cliff
- **Lens:** P + R (performance P-10, robustness P-45).
- **Problem:** After two failed compactions the agent simply stops with a "still exceeded" notice (`agent.rs:1967-1976`; `subsystem-reviews/core-loop-and-tool-dispatch.md` gap #6). A single very long tool result can wedge a session.
- **Proposal:** Add graduated fallbacks — drop oldest agent-visible turns, summarize more aggressively, externalize the largest tool result (BR-6/BR-7), or transparently route the turn to a larger-context model — before giving up.
- **Inspired by:** novel (progressive degradation); agents that swap to a larger window.
- **Affected code:** `crates/biorouter/src/agents/agent.rs:1964-2019`, `context_mgmt/mod.rs`.
- **Impact:** Medium–high — recovers sessions that currently dead-end.
- **Effort:** M.
- **Risk:** Dropping turns / switching model changes context/behavior; make it explicit to the model.

### BR-14: Validate and retry the compaction summary; don't summarize with the weakest model
- **Lens:** R (robustness P-44); `competitive-comparison/compaction-and-memory.md`.
- **Problem:** Compaction is one `complete_fast` call with no length/empty/format validation and no retry on junk output; the *fast* (cheapest/weakest, possibly smaller-context) model writes the memory the strong model then relies on, and its role is force-set to `Role::User` (a mislabel) (`subsystem-reviews/compaction-and-context-management.md` gaps #2/#9/#11).
- **Proposal:** Validate the summary (non-empty, mandated sections present) and retry once on failure; use the main (or a configurable mid-tier) model for compaction to protect fidelity; stop forcing `Role::User`; adopt a structured schema that preserves task IDs/plan/file state (OpenHands `TASK_TRACKING`, Cline Files section).
- **Inspired by:** OpenHands condenser (minimum-progress guard), Codex dual-strategy, Claude Code (main-model summary).
- **Affected code:** `crates/biorouter/src/context_mgmt/mod.rs:286-349,317-318`.
- **Impact:** Medium.
- **Effort:** M.
- **Risk:** Using a stronger model raises compaction cost.

### BR-15: Fix token accounting — per-provider tokenizer + system/tools in the cold-path estimate
- **Lens:** (`competitive-comparison/compaction-and-memory.md` implication #3 — worst-in-class token counting).
- **Problem:** `o200k_base` (OpenAI) is used for Claude/Gemini/Bedrock/Ollama alike, and the fallback estimate uses `count_chat_tokens("", &[msg], &[])` — no system prompt, no tool schemas, the two largest contributors (`subsystem-reviews/compaction-and-context-management.md` gaps #4-5). This makes the 0.8 threshold an uncalibrated guess for every non-OpenAI model.
- **Proposal:** At minimum, include the system prompt and tool schemas in the cold-path estimate; ideally add per-provider tokenizers or a model-callable budget (Codex `get_context_remaining`/`new_context_window`).
- **Inspired by:** Codex CLI (model-callable budget).
- **Affected code:** `crates/biorouter/src/context_mgmt/mod.rs:184-213`.
- **Impact:** High — calibrates the compaction trigger across all providers.
- **Effort:** M.
- **Risk:** Per-provider tokenizers add dependencies; keep tiktoken as fallback.

### BR-16: Concurrency guard on the check→compact→persist sequence
- **Lens:** R (robustness P-43).
- **Problem:** `total_tokens` is read at turn start and written at turn end with no session-level lock; two turns racing on one session (a scheduled firing + a user turn) could double-compact or lose a compaction (`subsystem-reviews/compaction-and-context-management.md` gap #12). Compounds with the missing single-turn lock (BR-33).
- **Proposal:** Serialize compaction under the per-session turn lock (BR-33) or a dedicated session mutex.
- **Inspired by:** novel (concurrency correctness).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:1432-1510`, `session/session_manager.rs` replace path.
- **Impact:** Medium.
- **Effort:** S–M (largely subsumed by BR-33).
- **Risk:** Low.

### BR-17: Unified, auto-promoted cross-session memory
- **Lens:** U (ux P-28); `competitive-comparison/compaction-and-memory.md` implication #6.
- **Problem:** Cross-session memory is **three disjoint stores** — chatrecall substring `LIKE` OR-match ranked only by recency, opt-in Knowledge KB, and conversation-ingest — with no shared index and no auto-promotion; "Soul" is opt-in per query, never auto-injected (`subsystem-reviews/state-awareness-and-version-control.md` gaps #5/#6).
- **Proposal:** Index chat with SQLite FTS5 (already available) instead of substring `LIKE`, and add Codex/Claude-style auto-distillation of finished sessions into a ranked, auto-injected memory file so lab know-how accumulates without the user asking.
- **Inspired by:** Codex (`~/.codex/memories/`), Claude Code (auto-load `MEMORY.md`), Gemini (review inbox).
- **Affected code:** `crates/biorouter/src/session/chat_history_search.rs:117-172` (FTS5 index); KB ingest; a memory-injection step in `prompt_manager.rs`.
- **Impact:** High — turns strong-but-disjoint stores into effective recall.
- **Effort:** L.
- **Risk:** Auto-injection can leak stale/irrelevant facts; needs ranking + user review.

---

## Hooks & guardrails

### BR-18: Revive read-only auto-approve + per-action risk grading (make SmartApprove ≠ Approve)
- **Lens:** P + R + U (performance P-39, robustness P-19, ux P-6); `competitive-comparison/safety-and-guardrails.md`.
- **Problem:** `PermissionInspector`'s `readonly_tools`/`regular_tools` sets are constructed empty with no setter, so the read-only short-circuit never fires; the LLM permission judge (`check_tool_permissions`) has **zero callers** — so `SmartApprove` is behaviorally identical to `Approve` and over-prompts on every read (`subsystem-reviews/guardrails-and-permissions.md` gaps #1/#2).
- **Proposal:** Populate `readonly_tools`/`regular_tools` from the extension manager's `read_only_hint` annotations (a free, no-LLM signal) so reads auto-pass; adopt OpenHands' per-action `security_risk` (LOW/MED/HIGH/UNKNOWN) + `ConfirmRisky(threshold, confirm_unknown)` for the smart tier rather than resurrecting the dead judge; delete the unreachable `check_tool_permissions` path.
- **Inspired by:** OpenHands (`security_risk` + `ConfirmRisky`, read-only exempt, fail-safe HIGH), Claude Code auto-mode.
- **Affected code:** `crates/biorouter/src/permission/permission_inspector.rs:106-188`, `agent.rs:348-351`, `permission/permission_judge.rs`, `extension_manager.rs` (annotation plumbing).
- **Impact:** High — makes the "smart" tier actually smart and stops over-prompting reads.
- **Effort:** M.
- **Risk:** Mis-annotated "read-only" tools that actually write — fail-closed on unknown, validate hints.

### BR-19: PreToolUse tool-input rewrite + let PostToolUse block + stop dropping hook context
- **Lens:** R + U (robustness P-26, P-27, P-28, ux P-33); `competitive-comparison/safety-and-guardrails.md` behind #1/#8.
- **Problem:** Hooks can only allow/deny/ask/inject; there is no rewrite path anywhere, so a hook cannot sandbox a path, redact a payload, or normalize a shell command (`subsystem-reviews/hooks-system.md` #7). PostToolUse/PostToolUseFailure are observe-only although the block decision is already computed (`subsystem-reviews/hooks-system.md` #2, `agent.rs:1845-1847`). And `HookInspector`/`tool_execution` read only `aggregate.decision`, so `additionalContext`/`systemMessage` returned by PreToolUse or PermissionRequest hooks are silently discarded (`subsystem-reviews/hooks-system.md` #1, `inspector.rs:62`, `tool_execution.rs:77`).
- **Proposal:** Add Codex's `PreToolUseOutcome` shape — optional `updated_input` applied before dispatch, plus `should_block`/`block_reason`/`additional_contexts`. Honor the computed PostToolUse decision (on block, feed the reason back as a corrective tool result and keep working). Consume and inject PreToolUse/PermissionRequest `additional_context`/`system_messages` like the SessionStart/UserPromptSubmit path.
- **Inspired by:** Codex CLI (`updated_input`), Gemini CLI (`tool_input`), Pi (`event.input` mutation), Claude Code (PostToolUse block + injects PreToolUse additionalContext).
- **Affected code:** `crates/biorouter/src/hooks/{outcome.rs,inspector.rs:57-88}`, `agents/tool_execution.rs:77`, `agent.rs:1411-1422,1845-1913`.
- **Impact:** High — turns hooks from a veto into a policy/auto-fix engine, reducing prompts.
- **Effort:** M.
- **Risk:** Rewritten input bypasses re-validation — document and consider re-running inspectors; a bad PostToolUse hook could wedge a turn (bound with the block-cap pattern).

### BR-20: Always-on non-bypassable catastrophic-command denylist
- **Lens:** R (robustness P-20); `competitive-comparison/safety-and-guardrails.md`.
- **Problem:** Dangerous-command detection is off by default (`SECURITY_PROMPT_ENABLED=false`) and, even enabled, only ever *asks* (`should_ask_user: true`) — so in `Auto` mode a user gets no command screening at all (`subsystem-reviews/guardrails-and-permissions.md` #3, `security/mod.rs:35-41,133-142`).
- **Proposal:** Add a small always-on, non-bypassable hard-block list for a handful of catastrophic patterns (`rm -rf /`, disk-wipe `dd`, fork bombs) that fires even in `Auto` mode and cannot be disabled by config.
- **Inspired by:** Claude Code / OpenCode deny-by-default catastrophic rules.
- **Affected code:** `crates/biorouter/src/security/{mod.rs,patterns.rs,security_inspector.rs}`.
- **Impact:** High — closes the "Auto mode = zero screening" hole.
- **Effort:** S–M.
- **Risk:** False positives block legitimate work — keep the list tiny and high-confidence.

### BR-21: Replace regex command scanner with an auditable policy engine
- **Lens:** R (robustness P-21); `competitive-comparison/safety-and-guardrails.md` best-in-class = Codex.
- **Problem:** The 40+-entry regex table is trivially evadable (`r''m -rf`, `$(printf …)`, env-var indirection, a different tool wrapper) with no argv parsing or path canonicalization — a signature scanner presented as a security control (`subsystem-reviews/guardrails-and-permissions.md` #4).
- **Proposal:** Parse argv and canonicalize paths, and move rules into a declarative, testable policy (Codex `execpolicy` Starlark `prefix_rule` with self-tests + `host_executable` pinning, or Gemini's tiered TOML with an admin tier). Lives outside the binary as config.
- **Inspired by:** Codex `execpolicy` (best-in-class), Gemini CLI TOML policy engine, OpenCode wildcard last-match-wins.
- **Affected code:** `crates/biorouter/src/security/{patterns.rs,scanner.rs,mod.rs}`.
- **Impact:** High — real command governance for a lab/UCSF deployment.
- **Effort:** L.
- **Risk:** New engine surface to get right.

### BR-22: Scan tool *output* on the main loop (injection + PII)
- **Lens:** R (robustness P-22); `competitive-comparison/safety-and-guardrails.md` behind #8.
- **Problem:** Guardrails (PII masking, `Block`, run_state HITL) run only on the Agent Drafter app socket; the CLI/GUI loop has no PII stage and never scans tool *output* — the classic prompt-injection vector for agents reading web/file content. `GuardrailStage::{ToolInput,ToolOutput,Output}` are declared but unused (`subsystem-reviews/guardrails-and-permissions.md` #6/#9).
- **Proposal:** Add a tool-*result* guardrail stage on the main loop that scans returned content for injection markers and PII/PHI (reusing the existing local `pii.rs`), masking or quarantining before it enters the model context.
- **Inspired by:** Claude Code protected paths, OpenHands; novel for the PII local-first angle.
- **Affected code:** `crates/biorouter/src/guardrails/{mod.rs:13-26,pii.rs}`, `agents/agent.rs` result path (`:981,1808`), `large_response_handler.rs`.
- **Impact:** High — the biggest injection surface is currently unguarded.
- **Effort:** M.
- **Risk:** False-positive masking could hide real data — make it opt-in / mode-gated.

### BR-23: Central secret-redaction boundary across all extensions
- **Lens:** R (robustness P-23).
- **Problem:** `.biorouterignore` lives inside the Developer MCP server only, so any other extension (compute, files, third-party MCP, a different shell wrapper) that reads `.env`/`secrets.*` bypasses it; default patterns also miss `.pem`, `id_rsa`, `.aws/credentials` (`subsystem-reviews/guardrails-and-permissions.md` #7).
- **Proposal:** Move ignore/redaction enforcement to a central boundary (the tool-dispatch or extension-manager layer) applied to every read-side tool, and widen the default deny set.
- **Inspired by:** Claude Code protected paths.
- **Affected code:** `crates/biorouter-mcp/src/developer/rmcp_developer.rs:1670-1704` (extract), `crates/biorouter/src/agents/extension_manager.rs` dispatch.
- **Impact:** High — one bypassable boundary today.
- **Effort:** M.
- **Risk:** Must not break legitimate reads of config files.

### BR-24: Per-directory / per-command-prefix permission scoping
- **Lens:** R + U (robustness P-24, ux P-5).
- **Problem:** `ToolPermissionStore` keys `AlwaysAllow` on `blake3(tool_name + exact-JSON args)`, so "always allow `shell`" is either exact-args reuse or a blanket whitelist of *all* future `shell` invocations, including dangerous ones. No "allow reads under this dir but not writes" (`subsystem-reviews/guardrails-and-permissions.md` #8). This drives approval fatigue.
- **Proposal:** Add scoped permission grants (tool + command-prefix or path-glob + operation class), matched last-wins, so a user can persist "allow `git` in this repo" without whitelisting arbitrary shell. Surface the scope choice in the confirmation card.
- **Inspired by:** OpenCode wildcard rules, Gemini tiered TOML, Claude Code allow/ask/deny rules.
- **Affected code:** `crates/biorouter/src/permission/{permission_store.rs:79-127,permission_inspector.rs}`, GUI `ToolCallConfirmation.tsx`.
- **Impact:** Medium–high — the single biggest lever on approval fatigue.
- **Effort:** M.
- **Risk:** Rule precedence bugs could over-grant; matching semantics must be conservative and well-tested.

### BR-25: Fix `unwrap()` panics in the permission store
- **Lens:** R (robustness P-25).
- **Problem:** `ToolPermissionStore` calls `.unwrap()` on `tool_call` and will panic if a `ToolRequest` carries an `Err` tool_call; inspectors guard with `if let Ok`, but the store does not (`subsystem-reviews/guardrails-and-permissions.md` #10, `permission_store.rs:81,99,122`).
- **Proposal:** Return a deny/error on a malformed request instead of panicking (fail-closed, not crash).
- **Inspired by:** novel (defensive correctness).
- **Affected code:** `crates/biorouter/src/permission/permission_store.rs:81,99,122`.
- **Impact:** Low — a crash-to-panic hardening.
- **Effort:** S.
- **Risk:** Low.

### BR-26: Output-size limits + untrusted framing on injected hook stdout
- **Lens:** R (robustness P-30).
- **Problem:** Raw stdout (UserPromptSubmit/SessionStart) and `additionalContext` are injected verbatim with no truncation — a hook emitting megabytes silently bloats/blows the context, and it is a prompt-injection surface (a project hook's stdout lands as a hidden user message) (`subsystem-reviews/hooks-system.md` #5).
- **Proposal:** Cap injected hook output size (truncate with a marker) and wrap it in explicit data-not-instruction framing.
- **Inspired by:** Codex `project_doc_max_bytes`, Claude Code lower-trust project files.
- **Affected code:** `crates/biorouter/src/hooks/outcome.rs:180-186`, `agents/agent.rs:1413-1422`.
- **Impact:** Medium.
- **Effort:** S.
- **Risk:** Low.

### BR-27: Matcher on tool_input content, not just tool name (+ cache compiled regexes)
- **Lens:** R (robustness P-31).
- **Problem:** Hook matchers only see the tool name, so "only guard `rm -rf`" or "only writes under `/etc`" is impossible — every shell command must run the full guard script; the regex is also recompiled every call (`subsystem-reviews/hooks-system.md` #8, `matcher.rs:21`).
- **Proposal:** Extend matchers to optionally match on `tool_input` fields (e.g. a command/path regex), and cache compiled regexes.
- **Inspired by:** Gemini CLI args-regex rules, Claude Code.
- **Affected code:** `crates/biorouter/src/hooks/matcher.rs:10-28`.
- **Impact:** Medium.
- **Effort:** M.
- **Risk:** Low.

### BR-28: Return aggregates from `fire()` hook events
- **Lens:** R (robustness P-29).
- **Problem:** Notification, SubagentStart/Stop, Pre/PostCompact spawn detached tasks and drop the `HookAggregate` entirely, so even a `systemMessage` is lost, there is no way to know a compaction/subagent hook ran, and fire-and-forget can outlive the turn and race shutdown (`subsystem-reviews/hooks-system.md` #3).
- **Proposal:** Await these hooks (or at least capture and surface their aggregate/errors), and join outstanding hook tasks at turn/shutdown boundaries.
- **Inspired by:** novel (observability + lifecycle correctness).
- **Affected code:** `crates/biorouter/src/hooks/mod.rs:258-271`, fire sites in `agents/`.
- **Impact:** Medium.
- **Effort:** M.
- **Risk:** Low–medium; awaiting adds latency to those lifecycle points.

---

## Loop & stuck detection

### BR-29: Staged soft-then-hard repetition stop + honest repetition reason
- **Lens:** R + U (robustness P-1, P-2, ux P-7); `competitive-comparison/safety-and-guardrails.md`.
- **Problem:** Repetition detection is a single hard deny at the 4th identical call with no soft nudge first, and on a `RepetitionInspector` denial the model receives the generic `DECLINED_RESPONSE` ("the user has declined…"), not the true "exceeded maximum repetitions" reason — actively misleading, so the model thinks the user refused (`subsystem-reviews/loop-and-stuck-detection.md` #1/#2, `agent.rs:757-766`, `tool_execution.rs:38-40`).
- **Proposal:** Emit a non-blocking soft warning at 3 identical calls (surface the REP-001 reason as injected guidance: "you have called this tool identically N times; change approach or stop"), escalate to hard `Deny` at 5, and stop mislabeling the block as a user decline.
- **Inspired by:** Cline / OpenCode (3-warn/5-stop `doom_loop`), Gemini CLI (three-layer).
- **Affected code:** `crates/biorouter/src/tool_monitor.rs:107-164`, `tool_inspection.rs`, the `DECLINED_RESPONSE` text in `tool_execution.rs:38-40`.
- **Impact:** High — fixes a correctness bug the model can never diagnose today and reduces false stops.
- **Effort:** S–M.
- **Risk:** Low; raising the hard threshold slightly loosens the guard, offset by the soft warning.

### BR-30: Semantic / near-duplicate / oscillation loop detection
- **Lens:** R (robustness P-4); `competitive-comparison/safety-and-guardrails.md` best-in-class = Gemini/OpenHands.
- **Problem:** `matches` requires byte-exact JSON and counts only *consecutive* calls, so a one-char arg change, an `A/B/A/B` oscillation, or a semantically-identical-but-textually-different call all bypass it (`subsystem-reviews/loop-and-stuck-detection.md` #1, `subsystem-reviews/state-awareness-and-version-control.md` #8). OpenHands detects alternating `[A,B,A,B]` (N=4) and action-error repetition (N=3); BioRouter detects neither.
- **Proposal:** Add heuristics to the inspector over the last ~20 events after the last user message: normalized-arg similarity (ignore ids/whitespace), alternating-pattern detection, and repeated action-*error* detection.
- **Inspired by:** OpenHands `StuckDetector` (5 heuristics), Gemini CLI.
- **Affected code:** `crates/biorouter/src/tool_monitor.rs`, `tool_inspection.rs`.
- **Impact:** High — catches the loop classes that actually occur in practice.
- **Effort:** M.
- **Risk:** Medium; heuristics can false-positive — gate behind soft warnings (BR-29) first.

### BR-31: Repeated-failing-result / no-progress detector
- **Lens:** R (robustness P-5).
- **Problem:** The inspector never looks at tool *results*; repeated identical error messages ("no such file" over and over), or a command that keeps failing the same way, are invisible. There is no "no file changed / no new information in N turns" detector outside `/goal` (`subsystem-reviews/loop-and-stuck-detection.md` #1, `subsystem-reviews/self-verification-and-doneness.md` #7).
- **Proposal:** Hash tool-result content (or its error signature) and track repeats; when the same failing outcome recurs N times, inject a "you are not making progress; change approach or ask the user" nudge and, on persistence, block.
- **Inspired by:** Gemini CLI content-chant detector, OpenHands action-error.
- **Affected code:** `crates/biorouter/src/tool_monitor.rs`, `agents/agent.rs` result collection at `:1792-1843`.
- **Impact:** High — closes the biggest honest loop-detection gap.
- **Effort:** M.
- **Risk:** Medium; needs careful result-normalization to avoid nuisance nudges.

### BR-32: Bring `/goal` stall detection to ordinary chat
- **Lens:** R (robustness P-6).
- **Problem:** The genuinely good progress-stall logic (Jaccard `reason_similarity`, `GOAL_STALL_LIMIT`, non-resetting iteration cap) lives only in the `/goal` Stop-hook loop and never runs for ordinary chat, which is where most stuck loops happen (`subsystem-reviews/loop-and-stuck-detection.md` #9, `subsystem-reviews/state-awareness-and-version-control.md` #8, `goal.rs:301-320`).
- **Proposal:** Factor the stall detector out of `goal.rs` and run a lightweight version at Stop-time for all sessions (a background "are you looping?" check on the transcript tail), periodic (e.g. after turn 30), not just when a goal is set.
- **Inspired by:** Gemini CLI periodic LLM loop check, BioRouter's own goal loop.
- **Affected code:** `crates/biorouter/src/agents/goal.rs:121-133,301-320`, `agents/agent.rs:2120-2233`.
- **Impact:** High — extends a mature primitive to the common case.
- **Effort:** M.
- **Risk:** Medium; an always-on LLM stall check adds cost/latency — make it periodic.

### BR-33: Server-enforced single-turn-per-session lock
- **Lens:** P + R + U (performance P-45, robustness P-33, ux P-16); "the single most important gap" in `subsystem-reviews/server-reply-flow-and-session-lifecycle.md`.
- **Problem:** There is no server-side one-turn-per-session guard; two concurrent `/reply` calls for one `session_id` share one `Arc<Agent>`, one `confirmation_rx`, and one `soft_interrupts` vec, interleaving turns and producing garbled/duplicated output; serialization is only client-side (`subsystem-reviews/server-reply-flow-and-session-lifecycle.md` gap #1, `reply.rs:257`, `manager.rs:84-116`). A raced/duplicate turn also doubles token spend.
- **Proposal:** Hold a per-session turn lock/queue server-side; a second `/reply` either queues or is rejected with "turn in progress."
- **Inspired by:** state-of-the-art agents (per-session turn lock).
- **Affected code:** `crates/biorouter-server/src/routes/reply.rs:257`, `state.rs`, `execution/manager.rs`.
- **Impact:** High — prevents shared-state corruption + wasted-token avoidance.
- **Effort:** M.
- **Risk:** Medium; a stuck lock could block legitimate new turns / deadlock on elicitation — needs a TTL/override.

### BR-34: Absolute per-tool call ceiling + tool-call (not just iteration) turn cap
- **Lens:** R (robustness P-7, P-9).
- **Problem:** `call_counts` tracks per-tool totals but is never read for any decision; a tool called hundreds of times with ever-changing args trips nothing except the loose 100-*iteration* cap, which counts provider round-trips — so a few iterations each firing dozens of parallel writes are unbounded (`subsystem-reviews/loop-and-stuck-detection.md` #5/#8, `tool_monitor.rs:46`, `agent.rs:1571-1583,1792-1843`).
- **Proposal:** Add a configurable absolute ceiling per tool (a tool run > K times per reply requires approval / is denied), reading the already-tracked `call_counts`; and add a per-reply total tool-call counter with a (higher) cap.
- **Inspired by:** Codex CLI (goal token budget); novel.
- **Affected code:** `crates/biorouter/src/tool_monitor.rs:46,61-65`, `agents/agent.rs:1571-1583`.
- **Impact:** Medium — a backstop the exact-duplicate guard misses.
- **Effort:** S.
- **Risk:** Low; set ceilings high enough not to bite normal work.

### BR-35: Global wall-clock / token / dollar budget per reply (with a progress meter)
- **Lens:** R + U (robustness P-8, ux P-14); `competitive-comparison/safety-and-guardrails.md` "Budget cap: no".
- **Problem:** Only the 100-turn iteration count bounds a reply; 429 backoff (~2 min/call) compounds inside it, so a throttled or pathological session can run far longer than a user expects with no wall-clock guard (`subsystem-reviews/loop-and-stuck-detection.md` #6, `subsystem-reviews/core-loop-and-tool-dispatch.md`).
- **Proposal:** Track cumulative wall-clock, tokens, and (if pricing known) dollars per reply; on exceeding a configurable budget, stop gracefully with a "budget reached, here's where I am — continue?" message, re-injecting `remaining_tokens` so the model wraps up. Show a live budget/turn meter in the GUI.
- **Inspired by:** Codex CLI (token budget + `budget_limit.md`), OpenHands (`max_budget_per_run` → `MaxBudgetReached`).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:1556-1583`, `agents/types.rs` (SessionConfig); token accounting in `session_manager.rs`; GUI stream state.
- **Impact:** High — bounds cost/time, not just iterations, and sets user expectations.
- **Effort:** M.
- **Risk:** Low; make it a graceful soft stop, not a hard kill.

### BR-36: Consolidate the two RepetitionInspector implementations
- **Lens:** R (robustness P-3).
- **Problem:** `check_tool_call` (stateful, mutates `last_call`/`repeat_count`) is only exercised by unit tests; production runs the stateless `inspect`, so `last_call`/`repeat_count`/`call_counts`/`reset()` are dead in prod, and `RetryManager::with_repetition_inspector` is never called — a future fix can land in the untested-in-prod path (`subsystem-reviews/loop-and-stuck-detection.md` #3/#10).
- **Proposal:** Delete `check_tool_call` (or make `inspect` delegate to a single shared core) and re-point the tests at the production path; delete or wire `RetryManager::with_repetition_inspector`.
- **Inspired by:** novel (dead-code hygiene).
- **Affected code:** `crates/biorouter/src/tool_monitor.rs:59-88`, `agents/retry.rs:63-83`, `tests/repetition_inspector_tests.rs`.
- **Impact:** Low — correctness/maintainability, prevents a latent trap.
- **Effort:** S.
- **Risk:** Low.

---

## Long-running & processes

### BR-37: Reap orphaned background shell jobs across restarts
- **Lens:** P + R (performance P-48, robustness P-14).
- **Problem:** Background shell jobs set `kill_on_drop(false)` and live in an in-memory per-`DeveloperServer` registry with no PID-file/parent-death reaping; a daemon crash orphans whole process groups forever with no way to discover or kill them — even though the llama.cpp sidecar already implements exactly this reaping (`subsystem-reviews/long-running-tasks-and-scheduling.md` #1, `background.rs:119`, `llamacpp_sidecar.rs:833-936`).
- **Proposal:** Reuse the sidecar's `run/<ppid>.pid` pattern: record background job PIDs to a run-dir file and sweep/kill children of dead parents on `DeveloperServer` start.
- **Inspired by:** BioRouter's own llama.cpp sidecar, Claude Code / Codex.
- **Affected code:** `crates/biorouter-mcp/src/developer/background.rs`, `rmcp_developer.rs:704`.
- **Impact:** Medium–high — prevents resource leaks and zombie processes.
- **Effort:** M.
- **Risk:** Low; key reaping on dead ppids only.

### BR-38: Reconcile `currently_running` on scheduler load
- **Lens:** P + R (performance P-48, robustness P-15).
- **Problem:** `load_jobs_from_storage` reinserts each job verbatim without resetting `currently_running`/`current_session_id`/`process_start_time`; a job mid-run at crash time reloads as running and is then *permanently skipped* by the overlap guard — a stuck-job bug on every crash (`subsystem-reviews/long-running-tasks-and-scheduling.md` #2, `scheduler.rs:512-548,175-178`).
- **Proposal:** One-line reconcile on load: force `currently_running = false` (and clear the session id / start time) for every loaded job.
- **Inspired by:** novel (crash-recovery hygiene).
- **Affected code:** `crates/biorouter/src/scheduler.rs:512-548`.
- **Impact:** Medium — fixes a silent permanent-skip after any crash.
- **Effort:** S.
- **Risk:** Low.

### BR-39: `shell_list` tool for background jobs
- **Lens:** R + U (robustness P-16, ux P-9).
- **Problem:** `job_id`s are ephemeral in-memory ints with no enumeration surface (`list()` exists but is `#[allow(dead_code)]`); if the agent forgets a `job_id` mid-session it cannot discover what it started (`subsystem-reviews/long-running-tasks-and-scheduling.md` #3, `background.rs:251`).
- **Proposal:** Surface the existing `list()` as a `shell_list` tool returning `[{job_id, cmd, status, new_output_available}]`.
- **Inspired by:** Claude Code / Codex "list background tasks".
- **Affected code:** `crates/biorouter-mcp/src/developer/{background.rs:251,rmcp_developer.rs}`.
- **Impact:** Low–medium.
- **Effort:** S.
- **Risk:** Low.

### BR-40: Async subagent handle + structured result envelope
- **Lens:** P + U (performance P-40, ux P-29).
- **Problem:** The parent `subagent` tool call blocks until the child finishes (`subagent_tool.rs:341-349`), and results are lossy — default `summary=true` returns only the last text message, yielding "No text content in last message" if the child ends on a tool call (`subsystem-reviews/long-running-tasks-and-scheduling.md` gaps #4/#5).
- **Proposal:** Add a spawn→poll model (`subagent_status`/`task_status`) and a typed result envelope `{status, summary, error, artifacts}` so a child ending on a tool call yields a meaningful result and a long subagent doesn't stall the parent turn.
- **Inspired by:** OpenCode (`task(background=true)` + `task_status`), Codex (`wait_agent`/`resume_agent`).
- **Affected code:** `crates/biorouter/src/agents/subagent_tool.rs`, `subagent_handler.rs:58-114`.
- **Impact:** Medium — real parallel delegation without lossy summaries.
- **Effort:** L.
- **Risk:** Lifecycle/cleanup of detached subagents; result delivery ordering + persistence.

### BR-41: Recover or cleanly fail pending elicitations, goals, and in-flight runs on restart
- **Lens:** R + U (robustness P-17, P-18, ux P-3).
- **Problem:** Goal state is in-memory only (`GoalRegistry` on the `Agent`), so a daemon restart silently drops an active `/goal` while todos (in `extension_data`) survive — an inconsistency (`subsystem-reviews/state-awareness-and-version-control.md` #3, `goal.rs:99-101`). Likewise `ActionRequiredManager`'s pending oneshots are in-memory; a restart drops them and any parked tool call is lost with no user signal (`subsystem-reviews/long-running-tasks-and-scheduling.md` #10, `action_required_manager.rs:17-31`).
- **Proposal:** Persist `GoalState` into `session.extension_data` (versioned key `goal.v0`) and reload on resume, exactly like todos. Persist pending elicitations/approvals (extend the `RunState` paused-approval pattern that already survives reconnects) and, on startup, surface "this run was interrupted" instead of silently hanging; make elicitation session-scoped rather than a process-wide singleton.
- **Inspired by:** BioRouter's own todo persistence + `run_state.rs`, OpenHands persisted event store.
- **Affected code:** `crates/biorouter/src/agents/goal.rs:99-101`, `extension_data.rs`, `action_required_manager.rs:17-31`, `guardrails/run_state.rs`, `agents/mcp_client.rs:254-285`.
- **Impact:** Medium.
- **Effort:** M–L.
- **Risk:** Concurrency and routing correctness.

### BR-42: "What is the agent running now" dashboard
- **Lens:** U (ux P-8).
- **Problem:** Background shell jobs, subagents, and scheduled runs are **three disjoint in-memory systems with no unified view** of what the agent is currently running (`subsystem-reviews/long-running-tasks-and-scheduling.md` gap #11).
- **Proposal:** Add a unified "active work" surface (HTTP route + GUI panel) listing background jobs (job_id, cmd, status), running subagents, and in-flight scheduled runs, with a kill/cancel affordance per item.
- **Inspired by:** Claude Code (`/tasks`, TaskStop), Gemini CLI (background PIDs surfaced).
- **Affected code:** new `crates/biorouter-server/src/routes/` endpoint aggregating `background.rs`, subagent registry, `scheduler.rs`; GUI panel.
- **Impact:** Medium — user can see and stop runaway/forgotten work.
- **Effort:** M.
- **Risk:** Registries are per-`DeveloperServer`/in-memory; aggregation needs a shared handle.

---

## Checkpoints & version control

### BR-43: Shadow-git checkpoints + `/rewind` (three-axis restore)
- **Lens:** R + U (robustness P-12, ux P-1); "the single starkest deficit" in `competitive-comparison/execution-and-verification.md`.
- **Problem:** There is no git checkpointing, no shadow git, no session-level undo. The only rollback is `text_editor`'s in-memory, per-file, per-process LIFO — it dies with the process, misses shell/`write_file`/other-extension writes, and offers no "revert the whole task" (`subsystem-reviews/state-awareness-and-version-control.md` #2/#3). Aggressive autonomy is intolerable without a safety net.
- **Proposal:** Snapshot the worktree into a **private git object DB in the app data dir** before/after each model step (no commits, no branch moves, no touching the user's `.git`), covering all writers, and expose Cline-style three-axis restore (files / conversation / both) plus a `/rewind` slash command and a GUI rewind affordance on each turn.
- **Inspired by:** OpenCode (private git-object DB), Cline (shadow-git, 3 restore modes), Gemini CLI / Claude Code (shadow-repo + rewind), Aider (commit-per-edit).
- **Affected code:** new module in `crates/biorouter/src/` (reuse `git2`, already in-tree for KB); `agents/agent.rs` turn boundary; `crates/biorouter-server/src/routes/session.rs`; GUI `ui/desktop/src/components/BaseChat.tsx` + a rewind control.
- **Impact:** High — closes the biggest safety-net gap vs current-gen agents.
- **Effort:** L.
- **Risk:** Snapshotting large worktrees per step can be slow/space-heavy; needs gitignore-aware excludes and size caps.

### BR-44: Persist and extend `text_editor` undo history
- **Lens:** R (robustness P-13).
- **Problem:** The `text_editor` undo stack is `Arc<Mutex<HashMap<PathBuf,Vec<String>>>>` created fresh per `DeveloperServer` process and never persisted; it only covers `text_editor` edits (`subsystem-reviews/state-awareness-and-version-control.md` #2/#3, `text_editor.rs:1052-1106`).
- **Proposal:** As an incremental step toward BR-43, persist per-path undo history to disk (or the session DB) and record shell-redirect / `write_file` mutations so `undo_edit` covers them.
- **Inspired by:** Aider, Cline.
- **Affected code:** `crates/biorouter-mcp/src/developer/{text_editor.rs,rmcp_developer.rs:698}`.
- **Impact:** Medium.
- **Effort:** M.
- **Risk:** Low–medium.

### BR-45: Session branching UX (fork/tree) with stable message ids
- **Lens:** U (ux P-15); `competitive-comparison/compaction-and-memory.md` best-in-class branching = Pi.
- **Problem:** BioRouter has only `diverged_from` and **renumbers positional message ids on every rewrite** (compaction/edit), which is fragile for stable references (UI anchors, patch protocol); there is no first-class branching UX (`subsystem-reviews/state-awareness-and-version-control.md` gap #10).
- **Proposal:** Add stable message ids (UUIDs, not positional) and a `/fork`/`/tree` branching UX so users can explore alternatives without clobbering history. (Stable ids also unlock the SSE patch protocol in BR-53.)
- **Inspired by:** Pi (`/tree`, `/fork`, `/clone`), Claude Code (rewind + worktrees).
- **Affected code:** `session_manager.rs:1810-1841` (synthetic ids), `session.rs` diverge/edit routes, GUI history view.
- **Impact:** Medium — better exploration/recovery affordances.
- **Effort:** L.
- **Risk:** Stable-id migration touches persistence and UI anchors broadly; dual-read for old sessions.

---

## Verification & done-ness

### BR-46: Fix Anthropic `finish_reason` so length-truncation continuation works
- **Lens:** R + U (robustness P-40, ux P-25); "the single most surprising correctness gap" in `subsystem-reviews/core-loop-and-tool-dispatch.md`.
- **Problem:** The native Anthropic streaming format never populates `ProviderUsage.finish_reason`, so the length-truncation auto-continue is dead code for the default provider — a response cut off at the output limit ends the turn **silently mid-sentence** (`subsystem-reviews/core-loop-and-tool-dispatch.md` #1, `formats/anthropic.rs:637-683`).
- **Proposal:** Read `stop_reason` from Anthropic's `message_delta` and map `max_tokens` → `finish_reason = Some("length")` so the existing bounded auto-continue at `agent.rs:2053` fires (the OpenAI-compat format already does this).
- **Inspired by:** BioRouter's own OpenAI format path.
- **Affected code:** `crates/biorouter/src/providers/formats/anthropic.rs:637-683`, `providers/base.rs:303`.
- **Impact:** High — eliminates silent truncated answers on the primary provider.
- **Effort:** S.
- **Risk:** Low; well-scoped, testable.

### BR-47: Auto post-edit diagnostics (LSP/`analyze`) feedback loop
- **Lens:** R + U (robustness P-47, ux P-17); best-in-class = Claude Code / OpenCode / Aider per `competitive-comparison/execution-and-verification.md`.
- **Problem:** `text_editor` writes **never trigger diagnostics**; `analyze` (tree-sitter) is a manual tool and the `LSP` tool "is listed as available but is not part of the developer extension's edit path" (`subsystem-reviews/self-verification-and-doneness.md` gaps #1/#3, `subsystem-reviews/state-awareness-and-version-control.md` #7). The agent only learns of breakage if it *chooses* to run tests.
- **Proposal:** On a successful `text_editor` write, automatically run diagnostics on the edited file(s) and feed failures back as an agent-visible message through a single bounded reflection channel (Aider's model, `max_reflections=3`). Optionally an enforced completion gate for interactive coding sessions.
- **Inspired by:** Claude Code / OpenCode (auto LSP after edit), Aider (lint/test reflection), OpenHands (critic refine).
- **Affected code:** `crates/biorouter-mcp/src/developer/` (edit path), a reflection counter in `crates/biorouter/src/agents/agent.rs` (builds on BR-19).
- **Impact:** High — a real edit→check→fix loop for the R/Python/Rust code BioRouter targets.
- **Effort:** M.
- **Risk:** Noisy diagnostics could derail the model; cap reflections and scope to edited files.

### BR-48: Make a done-ness gate available in interactive chat
- **Lens:** U (ux P-19); "the biggest gap" in `subsystem-reviews/self-verification-and-doneness.md`, best-in-class = Codex/OpenHands/Claude Code.
- **Problem:** Enforced verification exists **only for workflows** (`execute_success_checks`), is single-variant (`Shell`), and on failure **discards all progress** by resetting to initial messages. In interactive chat, "done" is whatever the model decides (`subsystem-reviews/self-verification-and-doneness.md` gaps #1/#5/#6).
- **Proposal:** Make the mature `/goal` Stop-hook + a shell success-check a default-capable done-ness gate in interactive chat; add non-`Shell` check variants (file-exists, output-contains, JSON-schema); on failure, surface *what failed* and iterate on the diff instead of resetting.
- **Inspired by:** Codex (evidence-based completion audit), OpenHands (critic + goal-judge), Claude Code (Stop-hook test gate).
- **Affected code:** `crates/biorouter/src/agents/goal.rs`, `retry.rs:191-218` (`SuccessCheck` enum), `agent.rs:2087-2150`.
- **Impact:** High — stops "done with a broken build."
- **Effort:** L.
- **Risk:** Default gates could over-run cheap chats; keep opt-in-by-default per session type.

### BR-49: Wire the dormant `structured_output` validate/re-prompt loop
- **Lens:** R + U (robustness P-48, ux P-24).
- **Problem:** `structured_output.rs` has fence-stripping, parse/validate, and a `reprompt_message` for the BRSDK `output_type` contract but **zero call sites**, so any app relying on `output_type` currently gets no enforcement (`subsystem-reviews/self-verification-and-doneness.md` #2, `agents/mod.rs:23`).
- **Proposal:** Wire `structured_output` into the agent loop for BRSDK `output_type` (validate the terminal message, re-prompt up to N times), mirroring the working `final_output_tool` path.
- **Inspired by:** BioRouter's own `final_output_tool` design.
- **Affected code:** `crates/biorouter/src/agents/structured_output.rs`, app agent loop in `routes/apps.rs`.
- **Impact:** Medium — a written safety net that does nothing today.
- **Effort:** S–M.
- **Risk:** Low; primitives are tested, only wiring is needed.

### BR-50: Self-critique / reflection pass on ordinary answers
- **Lens:** U (ux P-23).
- **Problem:** **Nothing re-reads the agent's own answer** for correctness, contradiction, or hallucination before returning it — despite the science-accuracy mandate in `system.md` (`subsystem-reviews/self-verification-and-doneness.md` gap #7). Judges exist only for `/goal` and permissions.
- **Proposal:** Add an optional, cheap self-consistency/critique pass (LLM-as-judge or a "verify claims against tool evidence" step) before finalizing, gated by task type or a user toggle, that can trigger one corrective loop.
- **Inspired by:** OpenHands (`CriticMixin`), Gemini CLI (verification pass on compaction).
- **Affected code:** `crates/biorouter/src/agents/agent.rs` done-ness path; reuse the goal-judge primitive.
- **Impact:** Medium — fewer confidently-wrong biomedical answers.
- **Effort:** M.
- **Risk:** Latency/cost; must be scoped and skippable.

### BR-51: Structured tool-error taxonomy
- **Lens:** U (ux P-18).
- **Problem:** Tool errors are an **unstructured `is_error` bool + text blob**; a `cargo build` failure and a success both arrive as text, with no retryable-vs-fatal distinction and no file:line propagation (`subsystem-reviews/self-verification-and-doneness.md` gap #4, `subsystem-reviews/state-awareness-and-version-control.md` §5).
- **Proposal:** Add a typed error envelope (`{ kind: transient|invalid_args|tool_failure|not_found, retryable, message, structured?: {file,line,...} }`) that tools can emit and the model can branch on, keeping a human-readable fallback.
- **Inspired by:** OpenHands / Gemini CLI (typed `functionResponse:{error}`).
- **Affected code:** `crates/biorouter/src/agents/tool_execution.rs:111-116`, `conversation/tool_result_serde.rs`.
- **Impact:** Medium — cleaner self-correction, fewer blind retries.
- **Effort:** M.
- **Risk:** MCP tools return opaque errors; taxonomy must degrade gracefully.

---

## Performance & tokens

### BR-52: Carry the agent's computed `TokenState` in the SSE event (kill per-token DB reads)
- **Lens:** P (performance P-13).
- **Problem:** The server calls `get_token_state()` on *every* `AgentEvent::Message`, running a `SELECT` on `sessions` plus a `COUNT(*) FROM messages` that grows linearly with conversation length — and the token counts were already computed in-process before the event was emitted (`reply.rs:356,363` → `session_manager.rs:1006`). Pure redundant disk work on the hottest path.
- **Proposal:** Attach the agent's computed `TokenState` to the event payload; drop the per-event DB read entirely, or at minimum drop `COUNT(*)` and cache last token-state in `AppState`, refreshing only on `Finish`.
- **Inspired by:** jcode (trusts provider-observed tokens).
- **Affected code:** `crates/biorouter-server/src/routes/reply.rs:158-181,356-472`, `crates/biorouter/src/agents/agent.rs`.
- **Impact:** High — removes 2 SQLite queries per streamed token, one growing with history.
- **Effort:** M.
- **Risk:** Event payload schema change; keep a fallback for older clients.

### BR-53: Streaming-pipeline throughput bundle (coalesce deltas, patch conversation, stable ids, off-thread render)
- **Lens:** P (performance P-14, P-15, P-16, P-19, P-20, P-21).
- **Problem:** Multiple compounding streaming costs: each partial-text delta is independently `serde_json::to_string`'d and pushed as its own SSE frame (`reply.rs:185`); `UpdateConversation`/`HistoryReplaced` re-send the entire conversation mid-stream (`useChatStream.ts:228`); synthetic ids are positional so any rewrite renumbers messages (`subsystem-reviews/state-awareness-and-version-control.md` #10); the GUI re-runs the full ReactMarkdown/KaTeX/Prism pipeline every token (`MarkdownContent.tsx:180-240`) and the TUI re-parses the whole message every token (`tui/app.rs:161-179`); message components aren't memoized (`BioRouterMessage.tsx:37`); and `pushMessage` rebuilds the array + `JSON.stringify`s to dedupe per token (`useChatStream.ts:128-169`).
- **Proposal:** Coalesce deltas on a ~50-100 ms frame timer into one SSE frame; emit message-level patches (needs stable per-message ids — see BR-45) instead of full-conversation resends; render a cheap plain-text preview while streaming and swap to the full pipeline on finish (add a `needs_redraw` gate + ~60fps clock in the TUI); `React.memo` the message components and compute tool-chain maps once; throttle `setMessages` to an rAF boundary and replace the `JSON.stringify` dedupe with an id/length check.
- **Inspired by:** jcode (drain ≤32 events → 1 frame, five-layer content cache, `needs_redraw` gate); BioRouter's own CLI `stream_coalesce`.
- **Affected code:** `crates/biorouter-server/src/routes/reply.rs:183-199,345-406`, `ui/desktop/src/hooks/useChatStream.ts`, `chatStreamStore.tsx`, `components/{MarkdownContent,CodeBlock,BioRouterMessage,BaseChat}.tsx`, `ProgressiveMessageList`, `crates/biorouter-cli/src/tui/{app.rs,mod.rs}`.
- **Impact:** High (perceived) — the #1 streaming-latency source; removes quadratic client-side work and streaming jank in GUI + TUI.
- **Effort:** L (bundle; several M/S sub-tasks).
- **Risk:** Throttling adds up to ~100 ms perceived latency (flush on `Finish`); memo comparators/patch protocol must be correct or updates drop.

### BR-54: Share MCP servers across agents/sessions (SharedMcpPool) and one daemon per app
- **Lens:** P (performance P-26, P-27).
- **Problem:** Each `Agent` builds its own `ExtensionManager` and spawns MCP child processes per agent (`agent.rs:236`, `extension_manager.rs:236,250-252`); up to 100 live agents × M stdio/uvx servers (each 40-150 MB), with no shared pool — the dominant RAM multiplier. Separately, `startBiorouterd` spawns a fresh daemon for *every* Electron window (`biorouterd.ts:115,172`, `main.ts:612,683`), even though the server is already session-keyed and singleton.
- **Proposal:** Introduce an `Arc<SharedMcpPool>` keyed by extension config so N sessions share M server processes; sessions attach to shared clients rather than spawning their own. Start one daemon and connect all windows to it.
- **Inspired by:** jcode (`mcp/pool.rs`, one process owns all sessions).
- **Affected code:** `crates/biorouter/src/agents/extension_manager.rs`, `execution/manager.rs`; `ui/desktop/src/biorouterd.ts`, `ui/desktop/src/main.ts`.
- **Impact:** Very high (RAM) — collapses the largest process/memory multipliers.
- **Effort:** L.
- **Risk:** Shared MCP state (working dir, per-session env) needs careful isolation; window lifecycle (which window kills the daemon) needs rework.
- **Note:** groups two closely-related RAM redesigns; can ship independently.

### BR-55: Don't block the first frame / first turn on full MCP boot
- **Lens:** P (performance P-22).
- **Problem:** The CLI drains the entire extension `JoinSet` before entering the TUI (`builder.rs:578,285`); the GUI runs `loadURL` only after `/status` polls ready (`main.ts:665→837`); and `/status` is gated behind `Scheduler::new()` + `load_jobs_from_storage()` + `soul::install()` all awaited before `TcpListener::bind` (`commands/agent.rs:44,68`).
- **Proposal:** Render the first frame and accept input while MCP registration and scheduler/soul init happen in the background; make the MCP pool lazy (`OnceCell` on first tool use); bind the listener before the heavy init.
- **Inspired by:** jcode ("do NOT block the first turn on MCP connection").
- **Affected code:** `crates/biorouter-cli/src/session/builder.rs`, `ui/desktop/src/main.ts`, `crates/biorouter-server/src/commands/agent.rs`.
- **Impact:** High (perceived) — the dominant startup latency.
- **Effort:** M.
- **Risk:** A tool called before its MCP server is ready must queue or error gracefully.

### BR-56: Cut per-turn history work (incremental fix_conversation, Arc-shared transcript, char-estimate trigger)
- **Lens:** P (performance P-11, P-12, P-29) + R (robustness P-39 — re-fix suffix per turn).
- **Problem:** `fix_conversation` runs a 7-pass normalization over the *entire* history every turn and `inject_moim` runs a second full pass (`conversation/mod.rs:164-200`, `moim.rs:43`); the agent deep-clones the entire message history 2-3× per turn (`agent.rs:1288,1137`, `reply_parts.rs:186`) and the reply route double-clones (`reply.rs:274,298`); and `check_if_compaction_needed` re-tokenizes the whole history with a cache-less `TokenCounter` allocated/dropped each call, running the real tiktoken BPE synchronously on the async runtime (`context_mgmt/mod.rs:184-199`). Also, `fix_conversation` runs once per reply, so inside the multi-turn loop the next provider call can receive two consecutive un-normalized assistant messages (`subsystem-reviews/core-loop-and-tool-dispatch.md` #2).
- **Proposal:** Cache the normalized prefix and only re-fix the suffix appended since last turn (idempotent; also fixes the per-turn pairing risk); thread `Arc<[Message]>` through read-only paths, cloning only on mutation; hold one shared `TokenCounter` on the agent/session and use a fast char/heuristic estimate (or `spawn_blocking`) for the trigger check, keeping the exact encode only for the actual compaction.
- **Inspired by:** jcode (lazily-rebuilt provider-message cache, Arc-shared transcript, char estimate for trigger).
- **Affected code:** `crates/biorouter/src/conversation/mod.rs:164-221`, `agents/{agent.rs,moim.rs,reply_parts.rs}`, `context_mgmt/mod.rs:184-199,282-330`, `crates/biorouter-server/src/routes/reply.rs`.
- **Impact:** High — turns per-turn O(N) history cost into O(delta) in long sessions and removes a synchronous full-history encode.
- **Effort:** M.
- **Risk:** Subtle invariant bug if the prefix is mutated by a later normalization pass; Arc ownership refactor / borrow-checker friction; heuristic estimate can mis-trigger (keep exact count for the real compaction).

### BR-57: Move blocking file/git/log I/O off the async runtime
- **Lens:** P (performance P-17, P-18).
- **Problem:** `RequestLog` does blocking `std::fs` open/write/rename on **every request and every stream chunk** (`providers/utils.rs:473-562,207`), interleaved between token batches — the highest-impact blocking-I/O finding (touches 100% of requests). Also synchronous syscalls on async workers in `text_editor` (whole-file read+rewrite), PDF/DOCX/XLSX parsers, scheduler `fs::write`, and knowledge `git2` commits stall the runtime.
- **Proposal:** Buffer `RequestLog` in memory and flush via `spawn_blocking` (or an `mpsc` logging task); wrap the remaining blocking file/git calls in `tokio::task::spawn_blocking` or switch to `tokio::fs`.
- **Inspired by:** novel (internal perf review).
- **Affected code:** `crates/biorouter/src/providers/utils.rs:473-562,207`; `crates/biorouter-mcp/src/developer/text_editor.rs`, `computercontroller/{pdf_tool.rs,mod.rs}`; `crates/biorouter/src/scheduler.rs`; knowledge `service.rs`.
- **Impact:** High — unblocks the runtime during streaming; smoother concurrency under multi-session load.
- **Effort:** M (RequestLog) + S–M (mechanical for the rest).
- **Risk:** Buffered logs could be lost on crash — flush on shutdown.

### BR-58: Bound tool parallelism and add write-side ordering
- **Lens:** P + R (performance P-32, robustness P-46); `competitive-comparison/execution-and-verification.md` worst-in-class dispatch.
- **Problem:** `select_all` over all approved tool futures (`agent.rs:1792`) has no concurrency cap and no cross-tool isolation, so an assistant message with many write-side calls (e.g. concurrent edits to the same file) runs them all at once with no ordering guarantees (`subsystem-reviews/core-loop-and-tool-dispatch.md` #8).
- **Proposal:** Add a configurable semaphore over dispatched tool futures (default 8, like subagents) and serialize write-side tools that target overlapping paths (Codex's exclusive write-lock model).
- **Inspired by:** Codex CLI (R/W-lock gating), Gemini CLI (scheduler ordering); mirrors the subagent `SUBAGENT_SEMAPHORE`.
- **Affected code:** `crates/biorouter/src/agents/agent.rs:708-745,1792-1843`.
- **Impact:** Medium — avoids thundering-herd on disk/network and corrupt concurrent writes.
- **Effort:** M.
- **Risk:** Too-low a cap slows legitimately-parallel read tools — scope serialization to write-side/overlapping paths.

### BR-59: Startup, allocator, dependency and refetch/recompute cleanups
- **Lens:** P (performance P-23, P-24, P-25, P-28, P-31, P-33, P-34, P-36, P-37, P-41, P-42, P-43, P-44).
- **Problem:** A cluster of independent efficiency wins: `settings.json` read+parsed ~20×/launch (`utils/settings.ts:41-51`); no Cargo optimization profiles and ~988 crates compiled unconditionally (23 AWS, 15 tree-sitter, 7 boa-JS, all PDF/DOCX); system allocator everywhere causes RSS creep on long-running `biorouterd`; a fresh `reqwest::Client` per session-restore/subagent-spawn pays repeated TLS/pool warmups (`agent.rs:1978`, `subagent_tool.rs:414`); Auto Visualiser inlines Mermaid (3.3 MB → 4.4 MB base64) into persisted history despite the CDN flag; `RepetitionInspector` and inspectors re-scan full history serially every turn (`tool_inspection.rs:85-117`); regexes recompiled per call and tree-sitter queries per file×3; BM25 index rebuilt from scratch per `kb_search` (`knowledge/store.rs:176-227`); provider metadata + `McpAppCache` rebuilt on read (`config_management.rs:361`, `routes/agent.rs:1024`); loop-level streaming retry missing (`anthropic.rs:273`); an always-on incremental same-tool-failure counter; and the GUI polls schedules/llama/session-rename/extensions with no change-detection.
- **Proposal:** Module-scope write-through cache for settings; feature-gate heavy deps behind Cargo features + add tuned profiles; wire `tikv-jemallocator` behind a `jemalloc` feature; share a static `reqwest::Client` (keyed by TLS config); default Auto Visualiser persisted figures to CDN/proxy assets; make inspectors incremental (rolling window) and concurrent; move regexes to `LazyLock` + cache `Arc<Query>` per (lang,kind); build the BM25 engine once and invalidate on write; hold provider metadata + `McpAppCache` in `AppState` behind `OnceCell`/`RwLock`; wrap the streaming path in bounded retry-with-jitter for transient errors; add an incremental same-tool+same-args failure counter that escalates after K failures; push schedule/llama updates over SSE + add change-detection before `setState`.
- **Inspired by:** jcode (feature-gated jemalloc, minimal deps, one HTTP pool, trusts provider tokens); BioRouter's own `BIOROUTER_AUTOVIS_CDN` + non-streaming `with_retry`; novel.
- **Affected code:** `ui/desktop/src/utils/settings.ts`; `Cargo.toml` (workspace + crate manifests); `crates/biorouter-server/src/main.rs`, `crates/biorouter-cli/src/main.rs`; `providers/{api_client.rs:208-227,factory.rs,anthropic.rs:273-313,retry.rs,utils.rs}`; `autovisualiser/common.rs`; `tool_inspection.rs`, `tool_monitor.rs`; `analyze/parser.rs`, `knowledge/{graph.rs,store.rs:176-227}`; `routes/{config_management.rs,agent.rs:1024,1049}`, `state.rs`; `ui/desktop/src/components/{SchedulesView,LlamaServerInlineCard,BottomMenuExtensionSelection}.tsx`, `hooks/useChatStream.ts`.
- **Impact:** Medium–high in aggregate (binary size, cold compile, RAM, redundant refetch/recompute, transient-error resilience).
- **Effort:** M (bundle; individually S–M).
- **Risk:** Feature-flag matrix can break less-common provider paths (needs CI coverage); jemalloc build issues on some targets (keep feature-gated); cache staleness (wire invalidation to writes); retrying a partially-streamed turn must not duplicate content (needs a resume/rollback point).

---

## UX & agent ergonomics

### BR-60: Structured, per-item todo list + a living plan artifact
- **Lens:** U (ux P-2, P-4).
- **Problem:** The todo tool is a **full-overwrite `String` blob** ("WARNING: completely replaces the existing content"), so there is no per-item state, no completion tracking the app can render, and accidental truncation is one bad model write away (`subsystem-reviews/state-awareness-and-version-control.md` gap #4). Plan mode is likewise a **one-shot prompt-rewrite** with no maintained plan the agent checks off (`subsystem-reviews/self-verification-and-doneness.md` gap #9).
- **Proposal:** Replace the blob with a structured `Vec<TodoItem { id, text, status }>` in `extension_data`, with add/update/complete ops (not replace-all), rendered as a live checklist in GUI + CLI and re-injected compactly via MOIM. Add a persistent plan artifact (reuse the same store) the agent updates as it works, with a plan-completion checkpoint at turn end and a GUI plan view; optionally a heuristic that suggests plan mode for multi-step requests.
- **Inspired by:** Claude Code / Cline (TODO tracking, maintained plan), OpenHands (goal-judge over plan).
- **Affected code:** `crates/biorouter-mcp/.../todo_extension.rs`; MOIM rendering in `extension_manager.rs:1509`; `crates/biorouter/src/prompts/plan.md`; new GUI todo/plan panel.
- **Impact:** High — task visibility is the user's main progress signal.
- **Effort:** M (todo) + L (plan artifact).
- **Risk:** Schema migration of existing `todo.v0` blobs; model must adopt the op-based tool; needs a single source of truth for "the plan."

### BR-61: Wire the orphaned `/interrupt` soft-interrupt to the desktop client
- **Lens:** U (ux P-10).
- **Problem:** The soft-interrupt route (`/interrupt` → `queue_soft_interrupt`) is a genuinely nice "inject mid-turn without cancel-and-resend" feature but is **orphaned**: no `#[utoipa::path]`, absent from `openapi.json`, not in the generated TS client, never called by the GUI (`subsystem-reviews/server-reply-flow-and-session-lifecycle.md` gap; `subsystem-reviews/core-loop-and-tool-dispatch.md` notes soft interrupts as a good property).
- **Proposal:** Annotate the route, regenerate the OpenAPI + TS client, and add a GUI affordance (a "steer" input while a turn runs) that posts to it.
- **Inspired by:** Pi (queued steering messages).
- **Affected code:** `crates/biorouter-server/src/routes/reply.rs:498-505`; `just generate-openapi`; `ui/desktop/src/hooks/chatStreamStore.tsx`.
- **Impact:** Medium — unlocks a differentiating UX that already exists in the core.
- **Effort:** S.
- **Risk:** Low; the core plumbing is already tested.

### BR-62: Reliable cancel — addressable "cancel this turn" endpoint, request-scoped confirmations with TTL, cancellation-aware waits, idempotent `/reply`
- **Lens:** P + R + U (performance P-46, P-47, robustness P-34, P-35, P-36, P-37, P-38, ux P-11, P-12).
- **Problem:** Cancellation only works by closing the SSE socket; `/agent/stop` only evicts the agent from the LRU while the in-flight reply task keeps its own `Arc<Agent>`, so it does **not** cancel a running turn, and there is no `session_id`-addressed cancel (`subsystem-reviews/server-reply-flow-and-session-lifecycle.md` gap #4, `agent.rs:695-710`). `confirmation_rx` is one mpsc per agent, not request-scoped, so a stale/duplicate `/action-required` POST can resolve the wrong pending request and a lost confirmation blocks the turn **forever** (no TTL, no expiry) (`subsystem-reviews/server-reply-flow-and-session-lifecycle.md` gap #2, `tool_execution.rs:171-173`). The wait isn't in a `select!` with the cancel token, so a programmatic `/agent/stop` won't unblock it (gap #3). Cancellation is also cooperative and boundary-only — a long in-process tool body ignoring the token runs to completion (`subsystem-reviews/loop-and-stuck-detection.md` #7). And with `sseMaxRetryAttempts: 1`, an SSE reconnect re-POSTs and starts a *second* turn (gap #8).
- **Proposal:** Give the server a per-session `CancellationToken` registry so `/agent/stop` (or a new `/agent/cancel`) actually trips the running turn's token. Key confirmations by request id with a TTL that emits a "prompt expired" tool result and unblocks the loop; `select!` the approval wait against the cancel token and TTL. Run long in-process tools on abortable tasks (or a killable child process). Attach a client-generated turn/idempotency id so a re-POST resumes/dedupes instead of duplicating.
- **Inspired by:** BioRouter's own `mcp_client` `select!` on cancel; OpenHands (STUCK breaks); standard idempotency-key pattern; novel.
- **Affected code:** `crates/biorouter-server/src/routes/{agent.rs:695-710,reply.rs:249,action_required.rs}`, `state.rs`; `crates/biorouter/src/agents/{tool_execution.rs:171-229,agent.rs:152-153,836-960,1228-1236}`; built-in tool bodies in `biorouter-mcp`; `ui/desktop/src/hooks/chatStreamStore.tsx:536-544`.
- **Impact:** High — removes a permanent-hang class, enables headless/programmatic cancel and multi-client control, and prevents duplicate turns on flaky networks.
- **Effort:** M–L (bundle).
- **Risk:** A premature TTL could deny a slow human (make it generous/configurable); forced aborts can leave partial tool state; must not double-cancel or leave the SSE task dangling.

### BR-63: Richer tool-confirmation card + reasoning-effort control
- **Lens:** U (ux P-31, P-34).
- **Problem:** The confirmation card carries an inspector warning string, but there is no consistent preview of *what the tool will do* (the diff of an edit, the exact shell command) for the approval decision (`subsystem-reviews/server-reply-flow-and-session-lifecycle.md` §1, `subsystem-reviews/guardrails-and-permissions.md` Q3). Separately, there is **no reasoning-effort or thinking-budget knob** at the loop level; the explore-vs-answer tradeoff is left entirely to the model (`subsystem-reviews/self-verification-and-doneness.md` §4).
- **Proposal:** For write-side tools, render a preview (file diff for `text_editor`, the resolved command for `shell`) plus any security/risk explanation in `ToolCallConfirmation` so users approve with context and click "always allow" less blindly. Add a per-turn effort control (quick / normal / deep) mapping to thinking-budget/temperature/exploration caps, where deep mode enables the self-critique pass (BR-50) and a done-ness gate (BR-48); expose as a GUI toggle and a slash flag.
- **Inspired by:** Cline / Claude Code (diff previews on edit approval), Claude Code / Codex (effort tiers), subagent `max_turns` (the only existing explore budget).
- **Affected code:** `ui/desktop/src/components/ToolCallConfirmation.tsx`; the `ActionRequired` payload in `tool_execution.rs:161-169`; `crates/biorouter/src/agents/agent.rs` loop config; `SessionConfig`; provider params in `providers/base.rs`.
- **Impact:** Medium — better-informed approvals + user control over the depth/latency tradeoff.
- **Effort:** M.
- **Risk:** Large diffs need truncation (payload size on SSE); provider support for thinking budgets varies — degrade gracefully.

---

## Promoted standalone proposals

These four were originally folded into broader clusters; they are distinct
enough (distinct problem, code, effort, risk) to stand alone, so they are listed
here in full. Three belong logically under Hooks & guardrails (BR-64, BR-65,
BR-67) and one under Loop & stuck detection (BR-66); they are placed here for
visibility, which is why those two category ranges are discontinuous.

### BR-64: OS-level sandbox for tool execution
- **Lens:** R (robustness P-32); `competitive-comparison/safety-and-guardrails.md` best-in-class = Codex.
- **Problem:** BioRouter has no process isolation at all — its guardrail is permission gating, so autonomy is bounded by prompt compliance and the currently-off regex scanner, not the kernel (`competitive-comparison/safety-and-guardrails.md` behind #3, `subsystem-reviews/guardrails-and-permissions.md`).
- **Proposal:** Adopt Codex's two-axis model (what is technically possible via OS sandbox vs when to ask via approval): macOS Seatbelt `sandbox-exec -p` with writable-roots injected and network denied, Linux Landlock+seccomp+bubblewrap, escalate-to-approval on a sandbox denial rather than hard-fail.
- **Inspired by:** Codex CLI (best-in-class), OpenHands (Docker/VM), Gemini CLI, Claude Code (Bash sandbox).
- **Affected code:** `crates/biorouter-mcp/src/developer/` shell exec, `crates/biorouter/src/security/`, spawn paths in `extension_manager.rs`.
- **Impact:** High — kernel-enforced bound on autonomy.
- **Effort:** L.
- **Risk:** High; platform-specific, can break legitimate tool access — needs careful writable-root config.

### BR-65: Managed/enterprise policy tier for guardrails and hooks
- **Lens:** R (robustness P-50); `competitive-comparison/safety-and-guardrails.md` "Managed/enterprise policy tier: no".
- **Problem:** Both permissions and hooks have only 2 config tiers (global + opt-in project), with no non-overridable admin layer — a lab/UCSF deployment cannot enforce "no writes outside the data dir / always ask on `rm`" governance (`subsystem-reviews/hooks-system.md` #12).
- **Proposal:** Add an admin/managed tier (ownership-verified, outside the binary) that wins over user/project config for both the command policy engine (BR-21) and hooks.
- **Inspired by:** Gemini CLI (Default < Extension < User < Admin, admin wins), Claude Code managed settings.
- **Affected code:** `crates/biorouter/src/hooks/config.rs:111-143`, `security/`, `permission/`.
- **Impact:** High for institutional deployment, low for solo users.
- **Effort:** L.
- **Risk:** Policy resolution + ownership verification must be tamper-resistant.

### BR-66: Mistake-streak / recoverable-failure handling
- **Lens:** R + U (robustness P-10, ux P-27); `competitive-comparison/safety-and-guardrails.md` best-in-class = Cline.
- **Problem:** There is no counter for consecutive `api_error` / `invalid_tool_call` / `tool_execution_failed`; a non-context provider error just ends the turn with a "please retry" string, and there is no "one more chance with a hint" pattern (`subsystem-reviews/core-loop-and-tool-dispatch.md` #5, `subsystem-reviews/state-awareness-and-version-control.md` #7/#8).
- **Proposal:** Add a `MistakeTracker` over the last N tool/provider outcomes: below a cap emit a recoverable error and continue; at the cap inject a one-shot recovery notice (resetting the counter) or stop with preserved state.
- **Inspired by:** Cline `MistakeTracker` + `onLimitReached` (best-in-class), Aider `reflected_message` (cap 3).
- **Affected code:** `crates/biorouter/src/agents/agent.rs:2020-2028`, `tool_monitor.rs`, new module.
- **Impact:** High — strictly better than a hard end-turn on transient failure.
- **Effort:** M.
- **Risk:** Medium; count only true failures so legitimate iterative work isn't flagged, and don't mask genuinely fatal errors.

### BR-67: Runtime observability / trace of loop-safety events
- **Lens:** R (robustness P-49).
- **Problem:** `observability::{ObsEvent,TraceBuilder,TraceProcessor}` has no emit sites and `tracing/mod.rs` is a stub, so there is no runtime trace of tool-failure rates, retry counts, repetition triggers, or repair-loop firings — an operator cannot audit whether the safety mechanisms are working (`subsystem-reviews/self-verification-and-doneness.md` #8).
- **Proposal:** Emit the (already redaction-safe) spans at loop-safety decision points (inspector denials, retries, compaction, stop-hook blocks, cancellations) so the robustness features are observable.
- **Inspired by:** novel (operability).
- **Affected code:** `crates/biorouter/src/observability/mod.rs`, emit sites in `agents/agent.rs`, `tool_monitor.rs`, `hooks/`.
- **Impact:** Medium — you cannot improve loop safety you cannot measure.
- **Effort:** M.
- **Risk:** Low; ensure spans never carry args/text (the model already forbids it).

---

## Appendix: lens-to-BR crosswalk

For traceability against the original three lens files (every source proposal is preserved above).

- **[proposal-lenses/performance.md](proposal-lenses/performance.md)** P-1→BR-59, P-2→BR-59, P-3→BR-2, P-4→BR-5, P-5→BR-5, P-6→BR-8, P-7→BR-6, P-8→BR-7, P-9→BR-12, P-10→BR-13, P-11→BR-56, P-12→BR-56, P-13→BR-52, P-14→BR-53, P-15→BR-53, P-16→BR-53/BR-45, P-17→BR-57, P-18→BR-57, P-19→BR-53, P-20→BR-53, P-21→BR-53, P-22→BR-55, P-23→BR-59, P-24→BR-59, P-25→BR-59, P-26→BR-54, P-27→BR-54, P-28→BR-59, P-29→BR-56, P-30→BR-59, P-31→BR-59, P-32→BR-58, P-33→BR-59, P-34→BR-59, P-35→BR-59, P-36→BR-59, P-37→BR-59, P-38→BR-3, P-39→BR-18, P-40→BR-40, P-41→BR-59, P-42→BR-59, P-43→BR-59, P-44→BR-59, P-45→BR-33, P-46→BR-62, P-47→BR-62, P-48→BR-37/BR-38.
- **[proposal-lenses/robustness.md](proposal-lenses/robustness.md)** P-1→BR-29, P-2→BR-29, P-3→BR-36, P-4→BR-30, P-5→BR-31, P-6→BR-32, P-7→BR-34, P-8→BR-35, P-9→BR-34, P-10→BR-66, P-11→BR-59, P-12→BR-43, P-13→BR-44, P-14→BR-37, P-15→BR-38, P-16→BR-39, P-17→BR-41, P-18→BR-41, P-19→BR-18, P-20→BR-20, P-21→BR-21, P-22→BR-22, P-23→BR-23, P-24→BR-24, P-25→BR-25, P-26→BR-19, P-27→BR-19, P-28→BR-19, P-29→BR-28, P-30→BR-26, P-31→BR-27, P-32→BR-64, P-33→BR-33, P-34→BR-62, P-35→BR-62, P-36→BR-62, P-37→BR-62, P-38→BR-62, P-39→BR-56, P-40→BR-46, P-41→BR-10, P-42→BR-11, P-43→BR-16, P-44→BR-14, P-45→BR-13, P-46→BR-58, P-47→BR-47, P-48→BR-49, P-49→BR-67, P-50→BR-65.
- **[proposal-lenses/ux.md](proposal-lenses/ux.md)** P-1→BR-43, P-2→BR-60, P-3→BR-41, P-4→BR-60, P-5→BR-24, P-6→BR-18, P-7→BR-29, P-8→BR-42, P-9→BR-39, P-10→BR-61, P-11→BR-62, P-12→BR-62, P-13→BR-1, P-14→BR-35, P-15→BR-45, P-16→BR-33, P-17→BR-47, P-18→BR-51, P-19→BR-48, P-20→BR-3, P-21→BR-6, P-22→BR-4, P-23→BR-50, P-24→BR-49, P-25→BR-46, P-26→BR-62(per-tool timeout thread)/BR-63, P-27→BR-66, P-28→BR-17, P-29→BR-40, P-30→BR-5, P-31→BR-63, P-32→BR-62, P-33→BR-19, P-34→BR-63.

Every source proposal from all three lens files now maps to a distinct BR entry
(BR-1…BR-67). Where a performance sub-item is a mechanical member of a bundle
(e.g. BR-53, BR-59), it is enumerated inside that entry's Problem/Proposal text
so it remains individually actionable.

## Related documentation

- [Agentic loop review](README.md) — the executive report this register belongs to: how the loop worked at review time and how it compared to eight other agents.
- [Agent-loop fix campaign](../agent-loop-campaign/README.md) — the plan of record that turned these 67 proposals into dependency-ordered waves.
- [Campaign commit log](../agent-loop-campaign/commit-log.md) — one line per commit, mapping each commit to the `BR-NN` it implements; the closest thing to a per-proposal status column.
- [Campaign outcome report](../agent-loop-campaign/outcome-report.md) — what landed, the test progression, and the honest caveat list.
- [Platform parity audit](../agent-loop-campaign/cross-platform/platform-parity-audit.md) — defines `BR-68`…`BR-70` and the `GAP-N` cross-platform gaps added after this register was written.
