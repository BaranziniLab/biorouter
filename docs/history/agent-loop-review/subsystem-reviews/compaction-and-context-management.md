# Compaction and context management — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It documents the summarize-everything compaction strategy — the 0.8 trigger, the visibility-flag mechanism that makes compaction non-destructive, token counting, tool-pair edge cases and persistence — and records thirteen gaps.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, whose findings were then implemented. Gap #1 (no verbatim window) was fixed by BR-10, gap #3 (the single over-window message dead end) by BR-11, gaps #4 and #5 (tokenizer and cold-path undercount) by BR-15, gaps #2, #9 and #11 (weak-model summary, no summary validation, the `Role::User` mislabel) by BR-14, and gap #12 (the compaction race) by BR-16 and BR-33. Read it as a record of why those changes were made, not as current documentation of the compaction code.
> **Audience:** developers working on the agent loop, context management, or session persistence.

"Context management" in this title means the compaction path only — how an over-long conversation is shrunk. How the system prompt and per-turn context are *assembled* is a separate subsystem with its own review, [Context injection and system prompt construction](context-injection-and-system-prompt.md). Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); the numbered items under "Gaps and weaknesses" are what sibling reviews and the [review README](../README.md) cite as `compaction.md gap #N` (the file's former name).

## Scope and files reviewed

The subsystem's owner files are `crates/biorouter/src/context_mgmt/mod.rs`,
`crates/biorouter/src/token_counter.rs`, `crates/biorouter/src/conversation/{mod.rs,message.rs}`,
`crates/biorouter/src/prompts/summarize_oneshot.md`, plus the trigger sites in
`crates/biorouter/src/agents/{agent.rs,reply_parts.rs,execute_commands.rs}` and persistence in
`crates/biorouter/src/session/session_manager.rs`.

> **Note.** Line numbers in the citations below are as-read at review time and have drifted since; treat them as pointers to the right function, not exact locations.

## Overview

BioRouter has a **single-shot, summarize-everything** compaction strategy (the same approach Goose used),
not a sliding-window / keep-recent-verbatim scheme. When the running conversation is projected to
exceed a fraction of the model's context window, the entire agent-visible history is fed to an LLM
that produces one long structured summary. The original messages are **kept in the DB but flipped
to agent-invisible** (still shown to the user); the summary + a short "keep going" continuation
note + a fresh copy of the user's latest message become the new agent-visible context.

Visibility is the core mechanism. Every `Message` carries `MessageMetadata { user_visible,
agent_visible }` (`conversation/message.rs:509-523`). The **provider format layer** filters
`is_agent_visible()` when serializing the request (e.g. `providers/formats/anthropic.rs:38`,
`providers/formats/openai.rs:63`, `google.rs:37`, `openrouter.rs:55`, `bedrock.rs:178`), so the
model only ever sees agent-visible messages. The UI renders user-visible ones. Compaction rewrites
these two flags rather than deleting content.

Data flow (auto path):

```text
reply() [agent.rs:1424]
  └─ load full Conversation from session (all messages, all metadata)
  └─ check_if_compaction_needed(provider, conv, None, session)  [mod.rs:168]
        current_tokens = session.total_tokens (provider-reported)   ── happy path
                       | tiktoken estimate over agent_visible msgs  ── cold path
        needs = current_tokens / context_limit > threshold(0.8)
  └─ if needs:
        emit inline "Performing auto-compaction..." + thinking spinner
        fire PreCompact hook
        compact_messages(provider, conv, manual=false)  [mod.rs:50]
            do_compact: format agent_visible msgs -> summarize_oneshot.md
                        -> provider.complete_fast(...)  (fast model)
                        -> progressive tool-response removal on overflow
            mark originals with_agent_invisible(); summary=agent_only();
            + continuation assistant note; + fresh copy of last user msg
        session_manager.replace_conversation(id, compacted)  ── DELETE+reinsert
        update_session_metrics(compaction=true) -> total_tokens := summary output
        fire PostCompact; yield HistoryReplaced; "Compaction complete"
  └─ reply_internal(final_conversation) -> agent tool loop
```

There is a **second, reactive** compaction inside the tool loop that fires when the provider
returns `ContextLengthExceeded` mid-turn (`agent.rs:1964-2019`), and a **manual** path via
`/compact`, `/summarize`, or "Please compact this conversation" (`execute_commands.rs:10-11`).

## Review questions answered

### How does compaction work: trigger threshold, summarize vs truncate vs drop, prompt?

**Threshold.** `DEFAULT_COMPACTION_THRESHOLD = 0.8` (`context_mgmt/mod.rs:13`), overridable with the
`BIOROUTER_AUTO_COMPACT_THRESHOLD` config/env param (`mod.rs:176-180`, `agent.rs:1447-1449`).
`check_if_compaction_needed` computes `usage_ratio = current_tokens / context_limit` and returns
`usage_ratio > threshold`; a threshold `<= 0.0` or `>= 1.0` **disables** auto-compaction
(`mod.rs:215-221`). `context_limit` comes from `provider.get_model_config().context_limit()`
(`mod.rs:182`), which resolves to the model registry window or `DEFAULT_CONTEXT_LIMIT = 128_000`
(`model.rs:8,674-679`).

**Where it fires.** Proactively at the top of `reply()` *after* the new user message is persisted,
*before* the tool loop starts (`agent.rs:1432-1438`). The compaction itself runs inside the returned
stream (`agent.rs:1442-1510`). Reactively on `ContextLengthExceeded` inside the loop
(`agent.rs:1964`), capped at 2 attempts (`compaction_attempts >= 2` → give-up message,
`agent.rs:1967-1975`).

**Summarize vs truncate vs drop.** There is **no truncation and no true dropping**:
- *Summarize:* `do_compact` (`mod.rs:286-349`) takes all `is_agent_visible()` messages, renders each
  via `format_message_for_compacting` (`mod.rs:351-428`), stuffs them into the
  `summarize_oneshot.md` system prompt, and calls `provider.complete_fast(...)` — i.e. the **fast
  model** if one is configured (`providers/base.rs:460-489`). The whole agent-visible history is
  collapsed into one message.
- *Truncate (of the summarizer's input only):* if the summarization request itself overflows,
  `do_compact` retries with `removal_percentages = [0, 10, 20, 50, 100]` (`mod.rs:296`),
  progressively deleting **tool-response** messages "middle out" via `filter_tool_responses`
  (`mod.rs:236-284`). After 100% removal still failing → hard error "context limit exceeded even
  after removing all tool responses" (`mod.rs:336-338`). Note this removes tool *responses* but
  leaves their `tool_request(...)` text in the summarizer input.
- *Drop:* nothing is deleted from history. Post-summary, originals are marked
  `with_agent_invisible()` but stay `user_visible` (`mod.rs:121-133`); they remain in the DB.

**Images** are not summarized as pixels — `format_message_for_compacting` renders them as
`"[image: {mime_type}]"` text (`mod.rs:357`), so visual content is effectively lost at compaction.

**Prompt.** `prompts/summarize_oneshot.md`. It is explicitly an *agent-facing* summary ("This
summary will only be read by you so it is ok to make it much longer than a normal summary",
lines 6, 20), asks for `<analysis>` reasoning and 9 mandatory sections (User Intent, Technical
Concepts, Files + Code, Errors + Fixes, Problem Solving, User Messages, Pending Tasks, Current Work,
Next Step). The full history is injected as `{{ messages }}` (`SummarizeContext`, `mod.rs:30-33`;
`render_global_file`, `mod.rs:311`). The summarization user turn is the fixed string "Please
summarize the conversation history provided in the system prompt." (`mod.rs:313-315`).

### How is the conversation continued after compaction? What does the model see / the user see?

After `do_compact` returns the summary, `compact_messages` builds `final_messages`
(`mod.rs:119-164`):
1. All original agent-visible messages → `with_agent_invisible()` (still user-visible)
   (`mod.rs:121-133`). The single exception is the most-recent user message when it is the last
   message: it becomes fully `invisible()` because a fresh copy is re-appended (`mod.rs:122-127`).
2. The summary message → `MessageMetadata::agent_only()` and its role is forced to `Role::User`
   (`mod.rs:322`, `mod.rs:135`).
3. A continuation **assistant** message (`agent_only()`) with one of three canned texts
   (`mod.rs:15-28,139-150`): `CONVERSATION_CONTINUATION_TEXT` (a user turn was preserved),
   `TOOL_LOOP_CONTINUATION_TEXT` (mid tool loop), or `MANUAL_COMPACT_CONTINUATION_TEXT`. All three
   say "Do not mention that you read a summary…just continue".
4. For non-manual compaction, a **fresh copy of the most recent text-only user message** is appended
   (`mod.rs:93-109,155-159`) so the model still has the user's live ask.

**What the model sees next turn:** only agent-visible messages, after `fix_conversation` cleanup
(`agent.rs:424`, `conversation/mod.rs:164-200`) and the provider's `is_agent_visible()` filter — so:
the summary (as a User-role message), the continuation assistant note, and the re-appended user
message. The verbose original history is invisible to it.

**What the user sees:** the original messages are untouched in the transcript (still `user_visible`);
the summary and continuation are `agent_only`, hence hidden from the UI. During compaction the user
gets inline system notifications: "Exceeded auto-compact threshold of 80%. Performing
auto-compaction..." then a "biorouter is compacting the conversation..." spinner
(`COMPACTION_THINKING_TEXT`, `agent.rs:71`), then "Compaction complete" (`agent.rs:1452-1497`). The
UI is told to reload via `AgentEvent::HistoryReplaced(compacted_conversation)` (`agent.rs:1490`).

### How is token counting done, and how accurate is it vs provider truth?

**tiktoken, one global encoding.** `token_counter.rs` uses `tiktoken_rs::o200k_base()` in a global
`OnceCell` (`token_counter.rs:11,185-195`) — the **OpenAI** o200k BPE, used for *all* providers.
There is **no per-provider tokenizer**. `count_chat_tokens` adds 4 tokens/message overhead and a
+3 "reply primer", and skips non-agent-visible messages (`token_counter.rs:121-157`). Tool requests
are hashed as `"{id}:{name}:{args:?}"` using Rust `Debug` formatting (`token_counter.rs:138-142`);
tool-schema token cost uses hardcoded OpenAI-function-calling magic numbers (`FUNC_INIT=7`,
`PROP_INIT=3`, `FUNC_END=12`, etc., `token_counter.rs:16-21,60-113`). Counts are cached by AHash
with a 10k-entry cap and crude eviction (`token_counter.rs:13,42-57`).

**The threshold check prefers provider truth.** `check_if_compaction_needed` uses
`session.total_tokens` (the provider-reported total from the last completed turn) when available and
only falls back to tiktoken on a session's first turn or for providers that do not report usage
(`mod.rs:184-213`). The tiktoken path is offloaded to `spawn_blocking` because BPE over the whole
history is CPU-bound (`mod.rs:196-209`).

**Accuracy gaps:**
- The fallback estimate calls `count_chat_tokens("", &[msg], &[])` per message
  (`mod.rs:202-207`) — i.e. with an **empty system prompt and no tools**. Real requests include the
  system prompt and full tool schemas (often thousands of tokens), so the estimate **undercounts**
  system + tools while **over-counting** the +3 reply primer once per message. It is only a coarse
  proxy.
- o200k tokenization is wrong for Claude/Gemini/Bedrock/etc. — different BPE, so the estimate drifts
  from provider truth for non-OpenAI models.
- Images cost a few tokens (`[image: mime]`) here vs hundreds–thousands at the provider.
- `usage_estimator::ensure_usage_tokens` (`providers/usage_estimator.rs:16-44`) fills only missing
  input/output counts with the same tiktoken counter; if the provider reports usage, that is trusted
  verbatim.

### Edge cases: tool_call/tool_result pairing across the boundary, images, in-flight turns

**tool_call/tool_result pairing.** Handled by `fix_conversation` → `fix_tool_calling`
(`conversation/mod.rs:307-399`), which runs on the agent-visible slice before every provider call
(`agent.rs:424`, `reply_parts.rs` via `prepare_reply_context`). It removes orphaned tool responses
(no matching prior request) and orphaned tool requests (no following response), tool content on the
wrong role, etc. Because compaction replaces the whole agent-visible history with summary +
continuation + a plain-text user message, there are no dangling tool pairs to survive the boundary;
`test_keeps_tool_request` (`mod.rs:520-555`) asserts the compacted conversation validates. Note
`filter_tool_responses` can leave a `tool_request(...)` whose response was pruned in the
*summarizer's text input*, but that is prose fed to the summarizer, not a real tool message, so no
pairing invariant is violated.

**Images.** Rendered to `[image: {mime}]` placeholder text for the summarizer (`mod.rs:357`); after
compaction the original image messages are agent-invisible so they are not resent. Visual
information is therefore lost unless the summarizer captured it in prose.

**In-flight turns.** Proactive/manual compaction happens before the tool loop begins. Reactive
compaction inside the loop sets `did_recovery_compact_this_iteration = true`, persists the compacted
conversation, sets `conversation = compacted_conversation`, and `break`s the inner stream
(`agent.rs:1999-2012`); the outer loop then avoids `exit_chat` and continues from the last user
message (`agent.rs:2084-2085`). Distinct from output-length truncation: a `finish_reason == "length"`
turn with no tool call is auto-continued with `TRUNCATION_CONTINUATION_MESSAGE`, bounded by
`MAX_TRUNCATION_CONTINUATIONS = 12` (`agent.rs:76-79,2053-2071`) — that is response truncation, not
context compaction.

### What happens when a single message alone exceeds the window?

There is **no special handling and no per-message truncation**. A giant message (e.g. a 400k-token
tool result) is included wholesale in the summarizer input. `filter_tool_responses` can only remove
*entire* tool-response messages middle-out, never truncate one — so a single oversized tool response
that is not removed, or an oversized *user/assistant text* message (which `filter_tool_responses`
never touches), will keep overflowing. When `do_compact` exhausts `[0,10,20,50,100]` it errors with
"context limit exceeded even after removing all tool responses" (`mod.rs:336-338`); at the loop level
the second `ContextLengthExceeded` yields "Unable to continue: Context limit still exceeded after
compaction. Try using a shorter message, a model with a larger context window, or start a new
session." (`agent.rs:1967-1975`). So an individual over-window message is a dead end — the system
tells the user to start over rather than head/tail-truncating the offending payload.

### Interaction with session persistence — full history or compacted?

The DB stores the **compacted state, which still contains the full original history** (flagged
agent-invisible) plus the summary and continuation. `replace_conversation` →
`replace_conversation_inner` does a `DELETE FROM messages WHERE session_id = ?` then re-inserts every
message of the compacted `Conversation` with its `metadata_json` (`session_manager.rs:1872-1913`).
Since compaction keeps the originals (just agent-invisible), the raw transcript is preserved on disk
for the user, but it is **not reconstructable to an agent-visible state** — the flip is one-way.

`get_conversation` loads **all** messages with their metadata every time (`session_manager.rs:1810-1841`),
and `get_session(id, true)` attaches them as `session.conversation` (`session_manager.rs:1629-1632`).
Post-compaction, `update_session_metrics(..., is_compaction=true)` resets
`session.total_tokens := summary output_tokens` (`reply_parts.rs:365-368`), so the next
`check_if_compaction_needed` reads the small post-summary number instead of the old large one. The
lifetime `accumulated_*` counters keep incrementing (compaction is a billed provider call and is
recorded like any turn, `reply_parts.rs:377-407`). `/clear` zeroes tokens and empties the
conversation (`execute_commands.rs:138-158`).

## Notable design choices (worth keeping)

- **Visibility flags instead of deletion** (`message.rs:509-577`) cleanly separate what the model
  sees from what the user sees, and let compaction be non-destructive to the transcript. This is a
  genuinely nice primitive.
- **Provider-truth-first token accounting** (`mod.rs:184-213`): using the provider's reported
  `total_tokens` and only estimating on the cold path is the right call — provider usage is the
  ground truth and avoids running BPE every turn.
- **Offloading tiktoken to `spawn_blocking`** (`mod.rs:196-209`) prevents stalling runtime workers on
  the cold path.
- **Progressive, middle-out tool-response shedding** (`mod.rs:236-284`) is a reasonable escape hatch
  when the summarizer request itself overflows.
- **Three-tier trigger** (proactive-at-0.8, reactive-on-overflow, manual `/compact`) with observable
  Pre/PostCompact hooks (`agent.rs:317-335`) gives good coverage and extensibility.
- **Preserving the latest user ask verbatim** (`mod.rs:93-109,155-159`) keeps the immediate intent
  intact through summarization.
- **`fix_conversation` shadow-map** (`conversation/mod.rs:164-200`) fixes only the agent-visible slice
  while keeping invisible messages positionally intact — well-tested (many cases in `mod.rs` tests).

## Gaps and weaknesses

These thirteen items fed the improvement phase. They are what other documents in this
review cite as `compaction.md gap #N`; the numbering below is that scheme and is stable.

1. **Summarize-everything, no recent-turn verbatim window.** Unlike Claude Code / modern coding
   agents that keep the last N turns verbatim and summarize only the older prefix, BioRouter collapses
   the *entire* agent-visible history — including the most recent tool outputs — into lossy prose
   (`mod.rs:290-305`). Only one plain-text user message survives verbatim (`mod.rs:155-159`); the
   latest tool results, diffs, file contents, and errors are only as good as the summary. This is the
   single biggest fidelity regression vs SOTA.

2. **The summarizer is the *fast* model.** `do_compact` calls `complete_fast` (`mod.rs:317-318`,
   `base.rs:460-489`). The cheapest/weakest model writes the memory the strong model then relies on,
   and the fast model may have a *smaller* context window, making `do_compact` overflow (and thus
   drop tool responses) more often than the main model would.

3. **No truncation of an individual oversized message.** A single huge tool result or user paste that
   alone exceeds the window is a hard failure (`mod.rs:336-338`, `agent.rs:1967-1975`) telling the
   user to start a new session. There is no head/tail truncation, no "…N tokens elided…" middle-out
   on a single payload, and no pre-emptive clamp on tool-result size before it enters history.

4. **Fallback token estimate omits system prompt and tools.** `check_if_compaction_needed` estimates
   with `count_chat_tokens("", &[msg], &[])` (`mod.rs:202-207`), i.e. no system prompt, no tool
   schemas — which in this agent are frequently the largest single contributor. The cold-path ratio
   is systematically understated, so first-turn / non-reporting-provider sessions can blow past the
   real limit without triggering compaction.

5. **Single OpenAI tokenizer for all providers.** o200k_base (`token_counter.rs:188`) is wrong for
   Claude/Gemini/Bedrock/Ollama/etc. Combined with (4) and image undercounting, the 0.8 threshold is
   a soft, model-inaccurate guess rather than a calibrated budget.

6. **Threshold reads *last turn's* token count.** The proactive check runs before the new turn, using
   `session.total_tokens` from the *previous* completed turn (`mod.rs:184`, `agent.rs:1432`). It does
   not include the just-added user message, the system prompt/tools, or the tool outputs this turn
   will generate. A single turn that adds a large tool result can overshoot the window between checks
   — which is exactly why the reactive path exists, but that path is a costlier recovery.

7. **Images and non-text content are dropped at compaction** (`mod.rs:357`) and undercounted for the
   threshold — vision-heavy sessions lose data and mis-budget silently.

8. **Whole-history rewrite on every compaction.** `replace_conversation_inner` does
   `DELETE` + re-`INSERT` of the *entire* message set (`session_manager.rs:1879-1900`), and
   `get_conversation` deserializes the full (ever-growing, mostly agent-invisible) history every turn
   (`session_manager.rs:1810-1841`). For long sessions this is O(n) I/O and JSON parsing per turn with
   no pruning of stale agent-invisible content. There is no archival/rollup — the DB row only grows.

9. **Summary quality is unguarded.** One `complete_fast` call, no length/empty/format validation, no
   retry on a junk or empty summary. The summary's role is *forced* to `Role::User` (`mod.rs:322`) —
   a workaround that mislabels an assistant-authored summary as a user turn (affects role alternation,
   caching, and any provider that treats user vs assistant memory differently).

10. **`filter_tool_responses` is a blunt, asymmetric heuristic.** It removes whole tool responses
    "middle out" (`mod.rs:260-284`) but leaves their `tool_request(...)` text, producing dangling
    request-without-response prose in the summarizer input, and it can jettison important
    early-or-recent context depending on parity. No relevance/size ranking.

11. **Reactive compaction can summarize a summary.** After the first compaction the agent-visible set
    is basically summary + continuation + user; a second `ContextLengthExceeded` re-summarizes that
    (`agent.rs:1998`), and the 2-attempt cap (`agent.rs:1967`) then bails. Repeated overflow degrades
    to summary-of-summary before giving up.

12. **No concurrency guard on the check→compact→persist sequence.** `total_tokens` is read at turn
    start and written at turn end; two turns racing on one session (e.g. schedule + user) could
    double-compact or lose a compaction, since there is no session-level lock around this flow.

13. **Manual `/compact` preserves *no* user message** (`mod.rs:94-109` gated on `!manual_compact`),
    so a user who compacts mid-thought loses their verbatim last message into the summary — arguably
    fine, but undocumented behavioral asymmetry vs the auto path.

## Related documentation

- [Context injection and system prompt construction](context-injection-and-system-prompt.md) — the other half of context handling: how the prompt and message array are assembled each turn.
- [Compaction and memory compared with other agents](../competitive-comparison/compaction-and-memory.md) — how this summarize-everything design measures against nine other coding agents.
- [State, awareness, todos, goals and version control](state-awareness-and-version-control.md) — the SQLite session schema and the `extension_data` state that survives compaction.
- [Master improvement proposals](../improvement-proposals.md) — the BR-NN proposals (BR-10, BR-11, BR-14, BR-15, BR-16) that these gaps became.
- [Wave 1 compaction report](../../agent-loop-campaign/wave-reports/wave-1-compaction.md) — what was actually built and merged in response.
