# Core agent loop and tool dispatch — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It walks the reasoning loop end to end — one reply turn, tool dispatch and result flow, the `Conversation` invariant, provider retries, oversized responses and streaming — and records ten gaps.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, whose findings were then implemented. Its headline finding, gap #1 (the Anthropic streaming format never setting `finish_reason`, leaving length-truncation continuation as dead code), was fixed by BR-46; the oversized-response gap #4 by BR-6; and the two-attempt overflow cliff, gap #6, by BR-13. The architecture narrative below is still broadly useful; the gap list is spent.
> **Audience:** developers working on the agent loop, tool dispatch, or provider integrations.

This is the longest of the ten subsystem reviews. Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); the numbered items under "Gaps and weaknesses" are what sibling reviews and the [review README](../README.md) cite as `core-loop.md gap #N` (the file's former name).

Contents:

- [Overview](#overview) — the loop in one diagram
- [How one reply turn works end to end](#how-one-reply-turn-works-end-to-end)
- [How the agent calls tools; how results get back](#how-the-agent-calls-tools-how-results-get-back)
- [Contexts across turns; the `Conversation` invariant](#contexts-across-turns-the-conversation-invariant)
- [Provider/API error retries; context-length-exceeded](#providerapi-error-retries-context-length-exceeded)
- [Oversized tool responses](#oversized-tool-responses)
- [Streaming and how partial events reach the client](#streaming-and-how-partial-events-reach-the-client)
- [Notable design choices](#notable-design-choices-worth-keeping) and [Gaps and weaknesses](#gaps-and-weaknesses)

## Scope and files reviewed

The subsystem is the reasoning loop that turns a user message into LLM calls, tool
calls, tool results, and final text. Primary code lives in
`crates/biorouter/src/agents/agent.rs` (the heart — 2,725 lines at review time),
`agents/tool_execution.rs`, `agents/reply_parts.rs`, `agents/retry.rs`,
`agents/large_response_handler.rs`, `agents/types.rs`, `providers/base.rs`,
`providers/anthropic.rs`, and the `conversation/` module.

> **Note.** Line numbers in the citations below are as-read at review time and have drifted since; treat them as pointers to the right function, not exact locations.

## Overview

The agent is a streaming, multi-turn loop. A single "reply" (one user submit)
may consume many LLM round-trips ("turns"/"actions"), bounded by `max_turns`
(default 100, `crates/biorouter/src/agents/agent.rs:69`). All state is a
`Conversation` (a `Vec<Message>`), persisted to SQLite via `SessionManager` and
mirrored in memory.

Text data-flow for one reply:

```text
reply(user_message) [agent.rs:1240]
  ├─ elicitation short-circuit / hooks (SessionStart, UserPromptSubmit) [1248-1332]
  ├─ slash-command execution [1344]
  ├─ persist user message + hook-context msg [1406-1421]
  ├─ auto-compaction check (token budget) [1432] ──► compact_messages [1478]
  └─ reply_internal(conversation) [1512]
       └─ prepare_reply_context [1527]  ──►  fix_conversation [conversation/mod.rs:164]
                                             prepare_tools_and_prompt [tool_execution.rs:113]
       loop {  [agent.rs:1556]                (turn budget, cancel, final_output checks)
         drain soft-interrupts [1589] ; inject MOIM [1596]
         stream = stream_response_from_provider(...) [1603]
         while chunk in stream {  [1628]
            categorize_tools(response) [1670]  ──► yield AgentEvent::Message (partial text)
            if tool requests: inspect → permission → dispatch (parallel) → collect results
         }
         record usage once [2035]
         if no_tools_called: truncation-continue / final_output / retry / Stop-hook [2044-2106]
         persist + conversation.extend(messages_to_add) [2108-2111]
       }
```

Streaming is real: provider Server-Sent Events (SSE) deltas become `MessageStream` items
(`(Option<Message>, Option<ProviderUsage>)`), the agent forwards each as an
`AgentEvent` to the SSE consumer (`routes/reply.rs`), which relays to the
Electron GUI over its own HTTP/WS channel.

## Review questions answered

### How one reply turn works end to end

Entry is `Agent::reply` (`agent.rs:1240`). It first handles special cases:
elicitation responses are submitted and the stream ends empty
(`agent.rs:1248-1271`); user-configured `SessionStart`/`UserPromptSubmit` hooks
may block or inject context (`agent.rs:1278-1332`); slash commands are executed
and may fully resolve the reply (`agent.rs:1344-1409`). The user message is
persisted (`agent.rs:1406`), any hook context is added as a hidden user message
(`agent.rs:1413-1421`), the session's `Conversation` is loaded
(`agent.rs:1424-1430`), and an auto-compaction check runs against the token
budget (`agent.rs:1432`, `check_if_compaction_needed`). If over threshold it
compacts before the loop (`agent.rs:1478`, `compact_messages`) and emits
`HistoryReplaced`. It then delegates to `reply_internal` (`agent.rs:1520`).

`reply_internal` builds a `ReplyContext` via `prepare_reply_context`
(`agent.rs:417`): it runs `fix_conversation` (see below), optionally injects an
`<explicit-resource-context>` block for `/skill`, `/ext`, `/kb` markers
(`agent.rs:438-449`), and computes `(tools, toolshim_tools, system_prompt)`
(`tool_execution.rs:113`). The core is a `loop` inside an `async_stream`
(`agent.rs:1556`). Each iteration ("turn"):

1. Cancellation check, final-output check, turn-budget check
   (`agent.rs:1557-1583`).
2. Drain mid-turn soft-interrupt user messages (`agent.rs:1589`).
3. `inject_moim` adds any "message of the moment" context
   (`agent.rs:1596`).
4. `stream_response_from_provider` starts the LLM call (`agent.rs:1603`,
   `tool_execution.rs:174`).
5. Consume the stream (`agent.rs:1628`). Each chunk carries an optional partial
   `Message` and optional `ProviderUsage`. `categorize_tools`
   (`agent.rs:1670`, `tool_execution.rs:262`) splits frontend vs backend tool
   requests and yields the filtered assistant message to the client
   (`agent.rs:1672`). A pure-text chunk (`num_tool_requests == 0`) is pushed to
   `messages_to_add` and streaming continues (`agent.rs:1676-1679`).
6. For tool requests: create one `Arc<Mutex<Message>>` response slot per request
   (`agent.rs:1681-1692`), run frontend tools (`agent.rs:1694`), run inspectors
   + permission gating (`agent.rs:1723-1745`), dispatch approved tools, and drive
   `handle_approval_tool_requests` for ones needing confirmation
   (`agent.rs:1767`). Results are collected from a merged stream
   (`agent.rs:1792-1843`), then `PostToolUse`/`PostToolUseFailure` hooks fire
   (`agent.rs:1848-1913`).
7. Thinking content is re-emitted as its own message (needed for Gemini/thinking
   models, `agent.rs:1928-1941`); each tool request (assistant) and its response
   (user) are appended to `messages_to_add` and yielded
   (`agent.rs:1943-1959`).
8. Usage is recorded exactly once per turn (`agent.rs:2035`). If no tool was
   called, the loop decides how to end: auto-continue a length-truncated turn
   (`agent.rs:2053-2071`), continue if a `final_output` tool is still pending
   (`agent.rs:2072-2083`), run workflow retry logic (`agent.rs:2087`), or set
   `exit_chat`. `messages_to_add` is persisted and merged into the conversation
   (`agent.rs:2108-2111`). On `exit_chat`, Stop hooks / `/goal` evaluation may
   force another turn (`agent.rs:2120-2233`).

The final assistant text is simply the last streamed text message(s); there is
no separate "finalize" step — the loop breaks and the consumer calls
`maybe_rename_session` (`agent.rs:2258`).

### How the agent calls tools; how results get back

Dispatch entry is `Agent::dispatch_tool_call` (`agent.rs:836`). It routes by
tool name: `platform__manage_schedule` (`agent.rs:855`),
`platform__ingest_conversation` (`agent.rs:872`), `final_output`
(`agent.rs:887`), `subagent` (`agent.rs:904`), frontend tools (returns an error
sentinel handled elsewhere, `agent.rs:938`), and finally the leaf MCP path
(`agent.rs:945-970`) which resolves `{{vault:NAME}}` secrets
(`agent.rs:952`, `apply_vault`) and calls
`ExtensionManager::dispatch_tool_call` (`extension_manager.rs:1228`). Every
result is post-processed by `large_response_handler::process_tool_response`
(`agent.rs:981`).

**Parallelism:** approved tools are dispatched eagerly into a
`Vec<(String, ToolStream)>` (`agent.rs:708-745`,
`handle_approved_and_denied_tools`), then merged with
`stream::select_all` (`agent.rs:1792`) and polled concurrently — so multiple
tool calls in one assistant message run in parallel. A `ToolStream`
(`agent.rs:194`) multiplexes MCP `ServerNotification`s and the final result
future via `tool_stream` (`agent.rs:201`). Tools **needing approval** are
handled serially in `handle_approval_tool_requests`
(`tool_execution.rs:53`): it yields an `ActionRequired` confirmation message and
blocks on `confirmation_rx.recv()` (`tool_execution.rs:171`) one request at a
time, pushing the dispatched future into the shared `tool_futures` on approval.

**Timeouts:** there is no per-call timeout in the agent loop itself, but the MCP
client enforces one. `await_response` (`mcp_client.rs:357`) `select!`s the
response against `tokio::time::sleep(self.timeout)` and the cancellation token;
on timeout it sends an MCP `CancelledNotification` and returns
`ServiceError::Timeout`. `self.timeout` is the extension's configured timeout,
default `DEFAULT_EXTENSION_TIMEOUT = 300` seconds
(`config/extensions.rs:11`). So the granularity is per-extension, not
per-tool; there is no way to bound a single slow tool below its extension's
timeout, and long-running local tools (shell, subagent) share that ceiling.

**Result shape / roles / serialization:** each request gets a response slot that
starts as `Message::user().with_id("msg_<uuid>")` (`agent.rs:1682`). When the
result arrives, `with_tool_response_with_metadata` writes a
`MessageContent::ToolResponse` into it (`agent.rs:1836`). The assistant's
`ToolRequest` message and the user's `ToolResponse` message are both pushed to
`messages_to_add` (`agent.rs:1943-1957`). So tool **requests are Assistant-role,
tool responses are User-role** — but `effective_role` treats a user message that
carries a tool response as role `"tool"` for merge purposes
(`conversation/mod.rs:427-436`). Serialization uses a tagged enum on
`MessageContent` (`conversation/message.rs:178`, `type` discriminant) and a
custom `ToolResult` serde that emits `{status:"success",value}` or
`{status:"error",error}` (`conversation/tool_result_serde.rs:7-26`); the
deserializer accepts multiple legacy shapes (`Vec<Content>`, value-arguments)
for backward compatibility (`tool_result_serde.rs:129-198`). Results are also
round-trip **validated** before being stored (`call_tool_result::validate`,
`tool_result_serde.rs:200`, invoked at `agent.rs:1808`) — a result that fails to
re-deserialize is replaced with an error so a malformed tool payload cannot
corrupt the persisted conversation.

### Contexts across turns; the `Conversation` invariant

`Conversation` is a newtype over `Vec<Message>` (`conversation/mod.rs:12`). Every
`Message` has `MessageMetadata { user_visible, agent_visible }`
(`conversation/message.rs:509`), which is the mechanism for hidden context:
hook context, explicit-resource context, and Stop-hook feedback are added with
`with_visibility(false, true)` (agent-visible, user-hidden), while system
notifications are `user_only()`. `Conversation::push`
(`conversation/mod.rs:44-63`) merges a message into the previous one when they
share an `id` — this is what stitches streamed text deltas (same `message_id`)
into one growing assistant message.

The invariant is enforced by `fix_conversation` (`conversation/mod.rs:164`),
called once per reply in `prepare_reply_context` (`agent.rs:424`). It builds a
shadow map so **only `agent_visible` messages are normalized**, and non-visible
messages keep their relative positions (`conversation/mod.rs:167-199`). The
normalization pipeline (`fix_messages`, `conversation/mod.rs:202-221`) applies,
in order: merge adjacent text content, trim trailing assistant whitespace,
remove empty messages, fix tool calling (drop tool requests/responses on the
wrong role and orphaned request/response pairs — `fix_tool_calling`,
`conversation/mod.rs:307-399`), merge consecutive same-`effective_role`
messages, drop a leading/trailing assistant message (`fix_lead_trail`,
`conversation/mod.rs:438-456`), and inject a placeholder `"Hello"` if the
conversation is empty (`conversation/mod.rs:458-468`). The resulting invariant:
**the agent-visible view starts and ends with a user turn, alternates user/tool
vs assistant, has no orphan tool requests/responses, and is never empty.**
`Conversation::new` validates by running the same pipeline and rejecting if it
would change anything (`conversation/mod.rs:125-136`).

Contexts accumulate by `conversation.extend(messages_to_add)` at the end of each
turn (`agent.rs:2111`) and by `SessionManager::add_message` for persistence.
Note `fix_conversation` runs **once per reply, not once per turn**, so messages
appended during the multi-turn loop are sent to the provider un-refixed until the
next `reply()` (see Gaps).

### Provider/API error retries; context-length-exceeded

There are **two independent "retry" mechanisms**, and `retry.rs` is *not* the
one that retries flaky API calls:

1. **Provider HTTP retry** lives in the provider layer. Anthropic wraps its POST
   in `self.with_retry(...)` (`providers/anthropic.rs:228`), from the
   `ProviderRetry` trait / `RetryConfig` in `providers/retry.rs`. This handles
   transient HTTP failures (rate limits, 5xx) for the *non-streaming* path. The
   streaming path (`anthropic.rs:273-313`) is **not** wrapped in `with_retry` —
   a mid-stream decode error becomes `ProviderError::RequestFailed` and ends the
   turn.
2. **Workflow success-check retry** is `retry.rs`'s `RetryManager`
   (`retry.rs:40`). It only activates when `session_config.retry_config` is set
   (a workflow feature). After a turn ends with no tool call, `handle_retry_logic`
   (`agent.rs:2087` → `retry.rs:112`) runs shell success checks
   (`retry.rs:191`); on failure it optionally runs an `on_failure` command,
   **resets the whole conversation to `initial_messages`**
   (`retry.rs:98-110`), clears the final-output tool, increments the attempt
   counter, and re-runs — up to `max_retries` (`retry.rs:131-155`). Both success
   checks and on_failure commands have mandatory timeouts (default 300s / 600s,
   `types.rs:16-19`, `retry.rs:221-269`).

When a non-context provider error surfaces in the loop, the agent does **not**
retry — it emits `"Ran into this error: … Please retry if you think this is a
transient or recoverable error."` and breaks (`agent.rs:2020-2028`), pushing the
retry decision onto the user.

**Context-length-exceeded** is special-cased in the stream match
(`agent.rs:1964`). `ProviderError::ContextLengthExceeded` triggers
`compact_messages` (`agent.rs:1998`); the compacted conversation replaces the
session history, `HistoryReplaced` is emitted, and the loop `break`s to restart
the turn on the smaller context (`agent.rs:1999-2013`). This is bounded: after
`compaction_attempts >= 2` it gives up with a user-facing "still exceeded"
notice (`agent.rs:1967-1976`). The Anthropic provider detects this error by
string-matching "too long"/"too many" in a 400 body
(`providers/anthropic.rs:143-157`).

### Oversized tool responses

Handled by `large_response_handler.rs`.

`process_tool_response` (`large_response_handler.rs:9`) runs on every successful
tool result (`agent.rs:981`). For each **text** content item whose
`chars().count() > LARGE_TEXT_THRESHOLD` (200,000 chars,
`large_response_handler.rs:6`), it writes the full text to a timestamped file
under `std::env::temp_dir()/biorouter_mcp_responses/`
(`large_response_handler.rs:62-77`) and replaces the content with a short pointer
message: *"The response … was larger (N characters) and is stored in the file
which you can use other tools to examine or search in: <path>"*
(`large_response_handler.rs:25-30`). If the file write fails it falls back to
inlining the full text with a warning (`large_response_handler.rs:32-40`).
Non-text content (images, etc.) and errors pass through unchanged
(`large_response_handler.rs:47-58`). The threshold is **per content item** and
counts characters, not tokens.

### Streaming and how partial events reach the client

Yes, streaming is first-class. `Provider::stream` returns a `MessageStream =
Stream<Item = Result<(Option<Message>, Option<ProviderUsage>), ProviderError>>`
(`providers/base.rs:668`). `supports_streaming` is true for essentially all
real providers (anthropic, openai, ollama, xai, google, databricks,
githubcopilot, openrouter, tetrate, zai, gcpvertexai, llamacpp, xiaomi_mimo…).
`stream_response_from_provider` (`tool_execution.rs:174`) calls
`provider.stream()` when supported, otherwise wraps a single `complete()` in
`stream_from_single_message` (`providers/base.rs:672`) — so the loop consumes a
stream uniformly.

For Anthropic, `stream()` opens an SSE POST (`providers/anthropic.rs:273-313`)
and `response_to_streaming_message` (`providers/formats/anthropic.rs:469`)
translates events: `content_block_delta`/`text_delta` yields a partial
assistant `Message` carrying just the new text with the shared `message_id`
(`formats/anthropic.rs:563-576`); `input_json_delta` accumulates tool arguments
(`formats/anthropic.rs:577-586`); `content_block_stop` yields the fully-assembled
tool call (`formats/anthropic.rs:590-635`); `message_delta` yields running usage
snapshots so a cancelled turn still records billed tokens
(`formats/anthropic.rs:637-679`).

Partial events reach the GUI because the agent forwards each streamed chunk's
filtered message as `AgentEvent::Message` immediately (`agent.rs:1672`), and
`Conversation::push` merges same-`id` text deltas client-side and in the stored
history. MCP notifications emitted during a tool call are surfaced as
`AgentEvent::McpNotification` (`agent.rs:1840`). The consumer (`routes/reply.rs`)
turns `AgentEvent`s into SSE frames.

## Notable design choices (worth keeping)

- **Visibility metadata as first-class context control.** The
  `agent_visible`/`user_visible` split (`message.rs:509`) cleanly separates what
  the model sees from what the user sees, and `fix_conversation` respects it via
  the shadow map (`mod.rs:167-199`). This is a clean way to inject hidden hook
  context and Stop-hook feedback without polluting the transcript.
- **`fix_conversation` as an idempotent normalizer.** Modeling the provider
  invariant (alternating roles, no orphan tool pairs, non-empty, user-bookended)
  as a pure, well-tested pipeline (`mod.rs:202`) is robust and easy to reason
  about; the round-trip test asserts idempotence (`mod.rs:534-541`).
- **Usage recorded once per turn from the last snapshot** (`agent.rs:1618-1626,
  2035`) — correctly handles providers that emit `usage` on multiple chunks and
  preserves billing for cancelled turns.
- **Tool-result validation before persistence** (`tool_result_serde.rs:200`,
  `agent.rs:1808`) prevents a malformed tool payload from making a session
  un-loadable later.
- **Soft interrupts** (`agent.rs:294-309, 1589`) let a user inject a message
  mid-turn at a safe boundary without cancelling and re-sending the whole
  context — a genuinely nice UX property.
- **Vault substitution only on the leaf MCP path** (`agent.rs:945-952`) keeps
  decrypted secrets out of the model context and out of subagent/frontend
  branches.
- **Bounded auto-continue on truncation and Stop-hook blocks** with explicit caps
  (`MAX_TRUNCATION_CONTINUATIONS`, `STOP_HOOK_BLOCK_CAP`) rather than unbounded
  loop injection — the top-of-file note (`agent.rs:80-91`) shows this was a
  deliberate correction of a past runaway-loop bug.

## Gaps and weaknesses

These ten items fed the improvement phase. They are what other documents in this
review cite as `core-loop.md gap #N`; the numbering below is that scheme and is stable.

1. **`finish_reason` is never set by the native Anthropic streaming format**, so
   the length-truncation auto-continue is effectively dead code for the default
   provider. `formats/anthropic.rs:637-683` reads `stop_reason` in `message_delta`
   only to merge usage and never populates `ProviderUsage.finish_reason` (which
   defaults to `None`, `base.rs:303`). The OpenAI-compatible format *does*
   propagate it (`formats/openai.rs:485-495`). Result: on Anthropic, a response
   cut off at the output-length limit ends the turn silently mid-sentence
   (`agent.rs:2053` never matches `Some("length")`). This is the single most
   surprising correctness gap.

2. **`fix_conversation` runs once per reply, not per turn.** Inside the multi-turn
   loop the agent appends a separate thinking message (`agent.rs:1934-1941`), then
   for each tool a separate assistant `ToolRequest` message and a user
   `ToolResponse` message (`agent.rs:1943-1957`), each with a fresh UUID.
   `Conversation::push` only merges same-`id` messages (`mod.rs:44-63`), so the
   next provider call within the same reply can receive **two consecutive
   assistant messages** (thinking + tool request) that were never re-normalized.
   Correctness then depends entirely on each provider's `create_request` grouping
   consecutive same-role messages; this is an implicit contract that should be
   made explicit or re-fixed each turn.

3. **Coarse, per-extension tool timeout only.** Every tool call inherits the
   extension's 300 s timeout (`mcp_client.rs:369`, `config/extensions.rs:11`).
   There is no per-tool budget, no adaptive timeout, and no cheap "this tool is
   taking a while" signal. A single slow tool blocks the whole turn (all other
   parallel tools' results are collected in one `select_all`, but the turn cannot
   advance to the next LLM call until the stream drains). State-of-the-art coding
   agents expose per-tool timeouts and partial/streamed tool output.

4. **Large-response handling is crude.** A 200,000-**character** threshold is
   ~50k tokens and is applied per content item, so several items each just under
   threshold still blow the context (`large_response_handler.rs:20`). The
   remediation — dump to a temp file under `std::env::temp_dir()` and tell the
   model to "use other tools to examine or search in" that path
   (`large_response_handler.rs:25-30`) — assumes the shell/file tools can reach a
   path *outside the session working dir* (and outside a `.biorouterignore`
   sandbox), which may not hold. There is no head/tail preview, no line-count
   summary, and no token-aware truncation. Modern agents inline a bounded preview
   plus a handle.

5. **No loop-level retry for streaming/provider errors.** A mid-stream decode
   error or any non-context `ProviderError` ends the turn with a "please retry"
   string (`agent.rs:2020-2028`); the streaming path is not covered by
   `ProviderRetry` (`anthropic.rs:273` has no `with_retry`). Transient network
   blips during a long stream therefore surface to the user instead of being
   retried with backoff.

6. **Context-overflow recovery is a hard 2-attempt cliff** (`agent.rs:1967`).
   After two failed compactions it simply stops. There is no progressive fallback
   (drop oldest turns, summarize more aggressively, or transparently switch to a
   larger-context model) — a very long single tool result can wedge a session.

7. **Argument coercion can silently corrupt data.** `coerce_tool_arguments`
   (`tool_execution.rs:76`) turns string args into numbers/bools based on the
   tool schema. For a `["string","number"]` union it will parse `"42"` into `42`
   (`tool_execution.rs:35-51`), which can be wrong for identifiers/zip
   codes/versions. This is a lossy heuristic applied uniformly to every tool call.

8. **Unbounded tool parallelism.** `select_all` over all approved tool futures
   (`agent.rs:1792`) has no concurrency cap and no cross-tool isolation, so an
   assistant message with many write-side tool calls (e.g. concurrent edits to
   the same file) runs them all at once with no ordering guarantees. Approval, by
   contrast, is fully serialized on a single `confirmation_rx`
   (`tool_execution.rs:171`), which can bottleneck multi-tool approvals.

9. **Cooperative cancellation with coarse checkpoints.** `is_token_cancelled` is
   checked at loop boundaries and between stream chunks (`agent.rs:1557, 1629,
   1798`); a long-running single tool only stops if it honors the cancellation
   token forwarded through `dispatch_tool_call`. There is no hard kill for a
   misbehaving in-process tool.

10. **The temp-file large-response path is not cleaned up** and its
    documentation string in tests (`stored in the file:`) does not match the
    production string (`stored in the file which you can use…`,
    `large_response_handler.rs:26` vs test `:137`), so the test's path-extraction
    branch is effectively dead — the file-content assertion never runs. Minor,
    but indicates the happy path is under-tested.

## Related documentation

- [Server reply flow and session lifecycle](server-reply-flow-and-session-lifecycle.md) — what sits in front of this loop: the SSE `/reply` route that drives it.
- [Guardrails, security and the permission system](guardrails-and-permissions.md) — the inspector gauntlet every tool request passes through inside step 6 of the loop.
- [Loop detection, repetition and stuck states](loop-and-stuck-detection.md) — the caps and cancellation paths that bound this loop.
- [Execution and verification compared with other agents](../competitive-comparison/execution-and-verification.md) — how this dispatch design measures against eight other coding agents.
- [Master improvement proposals](../improvement-proposals.md) — the BR-NN proposals (BR-6, BR-13, BR-46) these gaps became.
