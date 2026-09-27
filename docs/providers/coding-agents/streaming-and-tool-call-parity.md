# Streaming and tool-call parity for the coding-agent providers

> **What this is.** The root-cause analysis and the implementation plan for making the
> `claude_code` and `codex` providers stream like every API provider: live markdown as tokens
> arrive, and full tool-call parity — each tool call visible the moment it is made, its
> success or failure shown, its arguments and result expandable — exactly as when an API
> provider is selected. Grounded in a read of this tree, of get-bb/bb (commit `3f86b7c`,
> 2026-08-21, which already streams both vendors on subscriptions), and of the vendors' own
> wire protocols (Claude Code 2.1.235/2.1.238, Codex CLI 0.147.0/0.149.0).
> **Status:** Implemented for live streaming, mirrored tool calls, human approval, cancellation,
> and live steering. The original phase ledger remains below as historical design material. See
> [what shipped](#what-shipped) immediately below. This page is kept as
> the **design record**: every "today" claim in the analysis sections describes the tree
> *before* this work, and is retained for the reasoning rather than as a description of the
> running system. For the running system read
> [how it works](how-it-works.md#a-turn-streams-and-its-tool-calls-are-mirrored) and
> [the tool bridge](tool-bridge.md#the-mirror-how-a-bridged-call-becomes-a-visible-card).
> **Audience:** developers working on the coding-agent providers.

bb proves the premise: the same consumer subscriptions, the same CLIs, live streaming. The
gap is entirely on BioRouter's side, and it is two independent gaps — the providers never
ask the CLIs for streaming output (and discard the streamed frames the CLIs send anyway),
and a bridged tool call executes on a path that emits nothing the agent loop, the session
store, or the GUI ever sees.

## What shipped

Option A — the mirror — was built, followed by interactive approval and live steering. Everything
below this section is the analysis and the plan as written before that work, preserved as a design
record rather than a description of the current runtime.

**Done:**

- **Recorded fixtures** for both vendors, under
  `crates/biorouter/tests/fixtures/coding_agent/`, with their own README. Every decoder rule
  is tested against real vendor frames rather than against a fake's idea of them.
- **`ClaudeCodeProvider::stream()`** (`claude_code.rs:794`), invoking the CLI with
  `stream-json --include-partial-messages --verbose` and routing frames through
  `coding_agent/claude_stream.rs`, which diverts every `tool_use` content-block event away
  from the reused Anthropic decoder before it can mint an unmarked, dispatchable
  `ToolRequest`.
- **`CodexProvider::stream()`** (`codex.rs:1017`) over the push-based
  `coding_agent/codex_stream.rs` decoder, replacing the `absorb` fold. The protocol fixes
  found by this research landed with it: approvals answered in shapes the schema defines
  (`"decline"`, not the invalid `"denied"`, `codex.rs:424-445`), and usage read from
  `thread/tokenUsage/updated` with `turn/completed.usage` kept as the fallback.
- **The mirror marker** (`coding_agent/mirror.rs`): the reserved `biorouterProviderExecuted`
  key in the existing per-tool provider metadata, with `bridged` and `child` values, the
  pair builders, and the fail-safe "any mirrored content suppresses dispatch" predicate.
- **The agent loop's one new branch** (`agent.rs:7182-7202`): a message carrying mirrored
  content is persisted and yielded, never dispatched.
- **Unexpected Codex child-local events** are mirrored too, marked `child`
  (`codex.rs`) so an upstream isolation regression remains visible. Current children start
  with local model tools disabled and an isolated config home.
- **In-stream turn ceilings** in both providers (`claude_code.rs:891-902`,
  `codex.rs:649-659`), because the blocking path's timeouts wrap awaits the streaming path
  never reaches. The fixed ceiling they enforced then was removed on 2026-08-26. A turn now
  has no time limit unless `BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS` sets one, and the
  in-stream check applies that same optional limit.
- **Lead/worker forwarding** (`lead_worker.rs:410-412`): the pair streams only when both
  halves do.
- **No schema change was needed.** `metadata` was already a free-form object on
  `ToolRequest` and `ToolResponse` in the generated OpenAPI schema, so the marker rides the
  existing serialized shape and no client regeneration was required.

**Current notes:**

- **Interactive approval of a bridged call is implemented.** The call parks on the same
  session-scoped approval queue as an ordinary tool and is bounded below the child transport
  timeout.
- **A GUI label separating `bridged` from `child` cards.** The marker is persisted, but no
  component reads it yet, so a Codex `exec` card looks like any other card. This is phase 4's
  one visual change and it has not landed.
- **Live steering is implemented.** Claude accepts another stream-json user turn. Codex uses
  `turn/interrupt` followed by `turn/start` on the same thread because Codex 0.147 acknowledged
  native `turn/steer` without applying it to later model decisions. Dropping the stream remains the
  hard-stop backstop and reaps the child through `kill_on_drop(true)`.
- **The parity gate in CI** and the browser sweep (phases 6 and 7's remaining items).

## Summary

- **Why no streaming (1):** neither provider overrides `stream()` or `supports_streaming()`,
  so both inherit the trait defaults — `NotImplemented` and `false`
  (`crates/biorouter/src/providers/base.rs:932-945`) — and the agent takes the blocking
  branch, wrapping one final `Message` with `stream_from_single_message`
  (`crates/biorouter/src/agents/reply_parts.rs:256,291`).
- **Claude Code is invoked with `--output-format json`** — the literal `"json"` at
  `crates/biorouter/src/providers/claude_code.rs:631` — buffers all stdout until the child
  exits (`:366-375`), and `parse_result_object` drops every frame that is not
  `system/init`, `system/api_retry` or `result` via `_ => {}` at `:441`.
- **Codex's `absorb` keeps only `item/completed{agentMessage}` text and the turn terminal**;
  `_ => false` at `crates/biorouter/src/providers/codex.rs:466` discards
  `item/agentMessage/delta`, `item/reasoning/*`, `item/started`, and every
  `item/completed{mcpToolCall|commandExecution|…}` — the tool traffic included.
- **Why tool calls are invisible (1, continued):** a bridged `tools/call` runs on a
  different task (`crates/biorouter-server/src/routes/tool_bridge.rs:41-44`) through
  `BridgeGrant::call` → `ExtensionManager::dispatch_tool_call`
  (`crates/biorouter/src/providers/coding_agent/bridge.rs:284-369`), and nothing on that
  path yields an `AgentEvent`, persists a `Message`, or touches the session bus. Codex's
  built-in `exec`/`apply_patch` never even reach the bridge (`codex.rs:197-199`).
- **The fix (2):** implement `stream()` on both providers. Claude: rerun the existing
  argv builder with `"stream-json"` plus `--include-partial-messages --verbose` and reuse
  the existing Anthropic SSE decoder
  (`crates/biorouter/src/providers/formats/anthropic.rs:560-960`) on the unwrapped
  `stream_event` payloads — **text and thinking events only**: `tool_use` content-block
  events must be diverted away from the decoder, because its §6.2b flush mints an
  unmarked batched `ToolRequest` message and its own `PendingToolCall`
  (`anthropic.rs:546-556,840-846,958-960,680-689`) that the loop would dispatch (see
  phase 1). Codex: turn `absorb` into a yielding decoder over
  `item/agentMessage/delta` and friends. The agent loop, SSE route, store and React
  components already handle streamed `Message`s with stable ids — no UI rework for text.
  Both `stream()`s carry their own in-stream 30-minute deadline from day one — the
  blocking-path timeouts wrap awaits the stream path never reaches (see phases 1, 2, 5).
- **Tool-call parity (2, continued):** the recommended design is **mirror** — the provider
  stream emits already-resolved `ToolRequest` + `ToolResponse` message pairs (built from the
  child's own `tool_use`/`tool_result` and `mcpToolCall` frames, enriched from the bridge's
  call record) carrying a provider-executed marker that the loop honours by persisting and
  forwarding **without dispatching**. The existing `ToolCallWithResponse` /
  `ToolCallStatusIndicator` / `ToolCallArguments` components then render them unchanged,
  and `PendingToolCall` (already wired end-to-end) gives the skeleton card the moment the
  tool's name is known.
- **What parity is not achieved:** a bridged call that needs human approval stays refused
  rather than prompting (`bridge.rs:334-339`) — the hand-back design that would fix this is
  described and deliberately deferred — and Codex's sandboxed built-ins are shown as
  *child-executed* cards without having passed BioRouter's permission or privacy gates,
  because they never did (an existing, documented gap, now visible instead of silent).

## Why there was no streaming, before this work

The chain has four links, each cited:

1. **Trait defaults.** `stream()` returns
   `Err(ProviderError::NotImplemented("streaming not implemented"))` and
   `supports_streaming()` returns `false` (`crates/biorouter/src/providers/base.rs:932-945`).
   Neither `claude_code.rs` (impl at `:574-649`) nor `codex.rs` (impl at `:627-707`)
   overrides either. `LeadWorkerProvider` forwards `uses_tool_bridge`
   (`crates/biorouter/src/providers/lead_worker.rs:391-393`) but not `supports_streaming`,
   so even a future streaming provider wrapped in a lead/worker pair would stay blocking.
2. **The blocking branch.** `reply_parts.rs:256` reads
   `let streaming = provider.supports_streaming();`; the `false` arm calls
   `provider.complete(...)` and wraps the single result:
   `Ok((message, usage)) => Ok(stream_from_single_message(message, usage))` (`:291`).
3. **Claude Code parses one object.** `complete_with_model` passes the literal `"json"`
   (`claude_code.rs:628-633`); `run` buffers every stdout line until `child.wait()` returns
   (`:366-375`, 30-minute ceiling at `:386-401`); `parse_result_object` matches only
   `system/init`, `system/api_retry` and `result` — everything else hits `_ => {}` at
   `:441` — and builds one `Message` from `result["result"]` (`:487-491`). The
   `"stream-json"` axis of `base_args` exists (`:171-172` documents it as "the streaming
   path") but it is exercised only by one argv unit test (`:877-894`); the `#[ignore]`d
   live tests hand-assemble an equivalent argv rather than calling the builder (the
   `"stream-json"` literals sit in hand-built arg vectors at
   `crates/biorouter-server/tests/tool_bridge_routes.rs:250` and `:554`), so nothing
   outside that one unit test ever builds the streaming invocation through the real
   builder — which is why phase 1's live test must go through `command_for`.
4. **Codex ignores the deltas it is already receiving.** `absorb` (`codex.rs:419-466`)
   keeps `item/completed{agentMessage}.text` (`:427-433`) and `turn/completed` usage
   (`:436-439`); `_ => false` at `:466` drops `item/started`, `item/agentMessage/delta`,
   `item/reasoning/textDelta`, `item/mcpToolCall/progress` and all non-`agentMessage`
   `item/completed` frames. The provider's own captured fixture shows an `mcpToolCall`
   item arriving and contributing nothing (`codex.rs:1158-1200`).

The design comment at `crates/biorouter/src/agents/agent.rs:7046-7048` states the
load-bearing assumption outright: "This works because those providers are non-streaming, so
the whole child turn happens inside the awaited call and therefore inside this scope." The
bridge module already documents the rule a `stream()` must follow instead
(`bridge.rs:625-636`): read `active_bridge_url()` and spawn the child **inside `stream()`,
before returning the stream** — never from a poll — while the `BridgeLease` itself outlives
stream consumption because `Agent::reply` binds it as a loop-body local before the scope
(`agent.rs:7049-7069`).

### Before: Claude Code

```text
Agent::reply                    ClaudeCodeProvider              claude -p child            tool_bridge route (other task)
  | issue_tool_bridge (4820)         |                              |                          |
  | ACTIVE_BRIDGE_URL.scope( ... )   |                              |                          |
  | supports_streaming()==false ---->| complete_with_model (:609)   |                          |
  |   (reply_parts.rs:256)           |  command_for(.., "json")     |                          |
  |                                  |  spawn, kill_on_drop (:340) >| starts                   |
  |                                  |  buffer ALL stdout (:366)    | system/init              |
  |                                  |  timeout(30m, wait) (:386)   | MCP tools/call --------->| grant.call → dispatch
  |                                  |                              |<-- result (isError?)     | (NOTHING emitted)
  |                                  |                              | {"type":"result", ...}   |
  |                                  |<-- lines, exit               | exits                    |
  |<- ONE (Message[text], usage) ----| parse_result_object (:410)   |                          |
  | stream_from_single_message → one item; text appears all at once |                          |
```

### Before: Codex

```text
Agent::reply                    CodexProvider                   codex app-server           tool_bridge route
  | scope/lease as above             |                              |                          |
  | supports_streaming()==false ---->| complete_with_model (:660)   |                          |
  |                                  |  initialize/thread/turn ---->|                          |
  |                                  |  pump: absorb() (:419)       | item/agentMessage/delta  | (dropped, :466)
  |                                  |                              | item/started|completed   | (dropped unless agentMessage)
  |                                  |                              | MCP tools/call --------->| grant.call → dispatch
  |                                  |                              | item/completed{mcpToolCall} (dropped)
  |                                  |                              | turn/completed --------->| ends pump
  |<- ONE (Message[joined text], usage) (:684-688)                  |                          |
```

## Why tool calls were invisible, before this work

**The bridge executes the child's call inside the provider call and emits nothing.** A
bridged `tools/call` arrives on an axum handler that deliberately takes no `AppState`
because it "runs on a different task from the turn that issued the grant"
(`crates/biorouter-server/src/routes/tool_bridge.rs:41-44`); it runs `grant.call(call)`
(`:133`) → `dispatch_one` → inspectors → `ExtensionManager::dispatch_tool_call`
(`bridge.rs:295-369`). A grep of `bridge.rs` and `tool_bridge.rs` for
`AgentEvent|broadcast|notify|tx.send` finds nothing; the only side channel is a
`tracing::debug!` about dropped hook context (`bridge.rs:524-531`), and `BridgeGrant` is
"Deliberately a snapshot rather than a handle back to the `Agent`" (`:68-70`). The agent
loop therefore sees exactly one final text `Message` per turn, and the GUI sees no tool
cards at all. A call routed to `needs_approval` is refused, never parked:
"``{name}`` needs a person's approval, and this turn has no way to ask for one"
(`bridge.rs:334-339`), because the child is blocked on an HTTP response with no human
channel (`:266-270`). ⚠ **That half has since been fixed and this paragraph is history**:
#107 parks the call on a real, routable approval request — see
[the tool bridge](tool-bridge.md#a-call-needing-approval-is-put-to-a-person-and-the-call-waits-107).
The line numbers above are the pre-fix tree's.

**Codex's built-ins never even touch the bridge.** `exec`/`apply_patch` "cannot be switched
off … only the sandbox constrains" them (`codex.rs:197-199`), `web_search` is disabled
(`:202`), and the user's own `~/.codex/config.toml` MCP servers are merged in rather than
replaced (`:206-213`). Those calls surface only as the `item/started`/`item/completed`
notifications the provider currently drops, plus approval requests that are all answered
`{"decision":"denied"}` (`:403-415`) — a literal that is **not in either Codex approval
enum** (valid values are `accept`/`acceptForSession`/…/`decline`/`cancel`, per the 0.147.0
schema; the bug rides along into this plan's phase 2).

**Transcript flattening already carries stored tool traffic forward.** `transcript.rs`
renders persisted `ToolRequest`/`ToolResponse` content as `[called tool: …]` /
`[tool result: …]` lines inside `<conversation_history>`
(`crates/biorouter/src/providers/coding_agent/transcript.rs:53-70`, result bodies capped at
4,000 chars), and drops `SystemNotification` content entirely (`:72-75`). Two consequences
for the plan: (a) if child tool calls are persisted as ordinary request/response message
pairs, later turns' prompts include them with zero new code — which is what stops the child
re-running lookups; (b) any representation that is *not* `ToolRequest`/`ToolResponse`
content (a notification, a side-channel event) is invisible to the next turn's prompt.

**The one thing a streaming provider must not do** is surface the child's already-executed
calls as plain `ToolRequest` content: a `ToolRequest` in the current turn's response
`Message` goes through `categorize_tools` and is dispatched by the loop
(`agent.rs:7178-7205`; `categorize_tool_requests` at
`crates/biorouter/src/agents/reply_parts.rs:351-430` filters on content only, never
metadata), which would execute every tool a second time.

## How bb does it

bb's architecture, end to end: vendor child ↔ per-provider **bridge** process (JSON-RPC
over stdio) → one `thread/delta { threadId, deltas[] }` notification carrying semantic
deltas → a provider-neutral **delta assembler** in the runtime (mints all ids, owns every
timeline invariant) → append-only `events` table with a per-thread sequence → a WebSocket
`events-appended` signal → clients refetch `GET /threads/:id/timeline?afterSequence=` → a
provider-blind row projection → `TerminalOutputBlock` / `ToolCallDetailBlock` React bodies.
"The bridge knows the dialect, the runtime knows the timeline"
(bb `docs/provider-bridge-protocol.md:6-11`).

### The raw frames, per vendor

**Claude Code** (Agent SDK `query()` with `includePartialMessages: true`, bb
`plugins/provider-claude-code/src/bridge/sdk-session.ts:277`; on the plain CLI the same
switch is `--include-partial-messages`, which per `claude --help` "only works with --print
and --output-format=stream-json"). Text streams as `stream_event` envelopes wrapping raw
Anthropic Messages events — recorded verbatim
(bb `packages/provider-bridge-protocol/recordings/claude-code/turn-tools/provider→bridge.ndjson:44`;
all bb recordings live under `packages/provider-bridge-protocol/recordings/` — there is no
top-level `recordings/` directory):

```json
{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"2"}},"session_id":"d8785ba1-…","parent_tool_use_id":null,"uuid":"ec3afc0e-…"}
```

Thinking is the same shape with `thinking_delta` (plan-mode recording:
`"delta":{"type":"thinking_delta","thinking":"It","estimated_tokens":null}`) plus
`signature_delta` chunks. A tool call streams as `content_block_start{tool_use}` →
`input_json_delta` chunks → the complete **`assistant` frame** repeating the block with full
`input` (turn-tools line 18: `"content":[{"type":"tool_use","id":"toolu_01DELyv…","name":"Bash","input":{"command":"cat /tmp/bb-recording-ws/math.js",…}}]`) →
the result as a **`user` frame** keyed by `tool_use_id`
(line 25: `"content":[{"tool_use_id":"toolu_01DELyv…","type":"tool_result","content":"export function add(a, b) {…}","is_error":false}]` with the richer
`"tool_use_result":{"stdout":…,"stderr":"",…}` beside it). The terminal `result` frame
repeats the final text a third time and carries the authoritative usage. Duplication is
structural: one `assistant` frame **per content block**, sharing `message.id`, arriving
before that block's `content_block_stop` — a consumer must pick one source of truth per
block, never both. `is_error:true` can arrive with `subtype:"success"` (auth-failure
recording), so classification must consult `terminal_reason`/`api_error_status`.

**Codex** (`codex app-server`, JSON-RPC notifications). Text: `item/started{agentMessage}`
(empty text) → per-token `item/agentMessage/delta`
(bb `packages/provider-bridge-protocol/recordings/codex/turn-tools/provider→bridge.ndjson` seq 39:
`{"method":"item/agentMessage/delta","params":{"threadId":"01a021e4-…","turnId":"01a021e4-ff0a-…","itemId":"msg_0a7eeffd…","delta":"I"}}`) →
`item/completed` carrying the full final text (verified across all 17 recordings:
concatenated deltas equal the completed text exactly). A command execution:
`item/started{commandExecution, status:"inProgress", command, cwd}` →
`item/completed{status:"completed", aggregatedOutput:"…", exitCode:0, durationMs:0}` —
in the 0.149.0 recordings unified-exec commands open and close in the same millisecond and
**zero** `item/commandExecution/outputDelta` frames exist. Reasoning items in every
recording open and close **empty** (`"summary":[],"content":[]`, the raw item carrying only
`encrypted_content`) because bb never sets `summary` on `turn/start`; the
`item/reasoning/summaryTextDelta`/`textDelta` paths are exercised only synthetically.
Usage arrives in `thread/tokenUsage/updated.tokenUsage` — camelCase, one notification
**per model request**, each carrying a per-request `last` and a cumulative per-thread
`total` (turn-tools seq 81: `total.totalTokens` 19767 = `last`; seq 94 after the second
request: `total` 39660 while `last` is 19893). In the 0.149.0 recordings `turn/completed`
carries **no** usage field and no `turn/failed` frame appears — failure is
`turn/completed{status:"failed", error}`. Both are version-scoped observations, not
protocol facts: BioRouter's own in-repo fixture, documented as "captured from a live
`codex app-server`", shows `turn/completed` carrying snake_case usage
(`codex.rs:1156-1200`) — a surface/version that did emit it.

### The unified grammar, lifecycle and cadence

Every vendor event becomes one of ~28 delta kinds keyed by
`{providerItemId?, channel?, parentRef?}`
(bb `packages/provider-bridge-protocol/src/thread-delta.ts:93-97`): `item.open` (a pending
row), `item.textDelta`/`item.textClose` (streams are items too — Claude keys assistant text
by `channel:"assistant"`, thinking by `"thinking-<i>"`; Codex by its own item id),
`item.close` carrying the **full terminal shape** ("the terminal shape wins",
`thread-delta.ts:481-486`, which is how final text replaces the accumulated stream with no
duplication), `usage`, `turn.boundary`, `session.ended`. Item status is exactly
`pending → completed | failed | interrupted` (bb `packages/domain/src/provider-event.ts:31-36`);
a declined Codex approval maps to `interrupted` + `approvalStatus:"denied"`; a repeated
close for a settled provider id is deduped ("codex retries the terminal notification after
approvals", `thread-delta.ts:492-495`). Text deltas are coalesced per stream in a **100 ms**
trailing-edge window (`delta-assembler.ts:353`, `textDeltaFlushMs ?? 100`; 0 disables — the
tests and parity harness run at 0), with every non-text event acting as an ordering
barrier. Five grammar rules (`item/opens-before-delta`, `item/settles-once`,
`turn/starts-once`, `turn/settles-once`, `turn/known`) are enforced live at runtime intake
and offline by the conformance kit.

Parity is enforced with **recorded fixtures**: four NDJSON lanes per `<provider>/<cell>`
committed to the repo, a fake child replaying the provider lane through the real bridge and
assembler, ids interned by first appearance, timestamps blanked, LCS-diffed against a
PR-annotated allowlist whose stale entries fail, and per-cell counts pinned in
`packages/provider-bridge-protocol/recordings/row-counts.json` where
`unhandled`/`grammarDrops` may only go down. The
claude-code and codex `turn-tools` cells share one scenario description and both pin
`rows: 2` while events differ (13 vs 48) — the equal row count *is* the measured
"identical user-visible result".

### What does not transfer

bb trusts the child: Claude and Codex run their **own** native tools, bb's bridge merely
observes them on the wire and classifies them (`Bash→command`, `Read→fileRead`, …), and only
bb-injected tools are proxied back. BioRouter's compliance boundary is the opposite —
"a tool the child runs itself is invisible to Biorouter's inspectors, permission modes,
`.biorouterignore` and vault" (`bridge.rs:5-16`) — so the child's tools are switched off
(Claude) or sandboxed read-only (Codex) and every real call comes **into** BioRouter over
the HTTP tool bridge, executed behind Gate C, hooks, vault and the permission inspectors.
BioRouter's tool-call parity therefore cannot be built by copying bb's trust model or its
classification tables: it must come from **BioRouter's own bridge record plus the child's
item notifications**. Also not transferable: bb's `outputDelta` child-stdout channels (the
shell runs on BioRouter's side), the SDK settings posture (`settingSources:
["user","project","local"]` is the exact opposite of the security-relevant
`--setting-sources ""`), the SDK in-process MCP channel (BioRouter's relay is an HTTP
endpoint with the capability in the URL), and session resume/fork (BioRouter flattens
history per turn and keeps its transcript authoritative — see
[performance and limits](performance-and-limits.md)).

## Design options

All three options share the streaming substrate (phases 0–2 below); they differ in how a
tool call becomes visible. The axes come from the loop and store invariants:
mid-stream dispatch of every assistant `ToolRequest` (`agent.rs:7178-7205`), the §6.2b
re-split of unsigned turns into one assistant row per request (`agent.rs:1302-1364`) with
provider ids kept on at most one row (`:285-300`), transcript flattening
(`transcript.rs:53-70`), Gate C living only inside `ExtensionManager::dispatch_tool_call`
(`crates/biorouter/src/agents/extension_manager.rs:2598-2604`), the refused approval path
(`bridge.rs:334-339`), the UI's status derivation
(`ui/desktop/src/components/ToolCallWithResponse.tsx:908-914`: response present → error?
`error` : `success`; else turn active → `loading` : `interrupted`) and later-message
response scan (`ui/desktop/src/components/BioRouterMessage.tsx:139-145`), the pending-card
contract (`ui/desktop/src/hooks/chatStreamStore.tsx:985-994` removes a skeleton only when a
`toolRequest` with the same id lands), cancellation by dropping the provider future with
`kill_on_drop(true)` (`claude_code.rs:340`, `coding_agent/appserver.rs:101`), the
`Agent::reply` stack cliff (`agent.rs:855-862`: new logic must be free functions, never
inline in the generator), and the repo-grep assertions that pin call-site counts for
`raise_privacy`, `floor` and `.call_tool(`.

### Option A — mirror (recommended)

The provider stream emits **already-resolved pairs**: an assistant `Message` holding the
`ToolRequest` (yielded when the child's tool call is known) and a user `Message` holding the
matching `ToolResponse` (yielded when the result is known), both carrying a
**provider-executed marker** — carried **only at the tool level**, as a reserved key in the
existing serialized `ToolRequest`/`ToolResponse` provider `metadata`
(`crates/biorouter/src/conversation/message.rs:60,65-76,97`) — that the loop honours by
persisting and forwarding **without dispatching**. Deliberately **not** a
`MessageProvenance` variant: that type is BR-71's security-purposed cross-session stamp
("Cross-session control without provenance is indistinguishable from prompt injection",
`message.rs:588-589`), with consumers that give "has provenance" a specific meaning —
`is_provenance_boundary` in merges (`crates/biorouter/src/conversation/mod.rs:599`) and
the subagent surfaces (`crates/biorouter/src/agents/subagent_handler.rs:451,1037,2059`) —
and an unknown `kind` deliberately "degrades to `None`" (`message.rs:608-610`), so an
older CLI/daemon reading the session would silently strip the stamp. That degradation
caveat is fail-safe here only because the marker is honoured on the **live stream path
only** — persisted rows are never re-dispatched — so a stamp lost by an older reader
degrades display attribution, never safety; this is stated as a design invariant, not left
implicit. Sources: on Claude, the `assistant` frame's `tool_use` block opens
the pair and the `user` frame's `tool_result` (same `tool_use_id`) closes it; on Codex,
`item/started{mcpToolCall}` opens and `item/completed{mcpToolCall}` closes — but the
Codex shapes are **schema-read only** (the 0.147.0 schema gives the item `arguments`,
`result` and `error`; no bb recording contains any `mcpToolCall` frame, and the only
in-repo sequence is a hand-labelled unit fixture whose item carries just
`id/server/tool/status`, `codex.rs:1170-1174`), so phase 0's capture gates the Codex half
of phase 3. The bridge additionally keeps a
per-turn call log on the `BridgeLease` (request id, post-hook-rewrite args, result, error,
duration) that the decoder uses to enrich or backfill — but the child's stream is the single
source for card identity and ordering, so bb's "seen twice" dedup problem never arises.
`PendingToolCall` is emitted at `content_block_start{tool_use}` / `item/started` with the
**same vendor call id**, so the existing skeleton card appears instantly and is replaced
when the request row lands. The `mcp__biorouter__` prefix is stripped so the card shows the
BioRouter tool name the API path would show (`tool_bridge_routes.rs:319` vs `bridge.rs:302`).

```text
claude child                stream() decoder                     Agent::reply loop                 GUI
 content_block_start(tool_use id=T) ─► yield (None,None,Pending{id:T,name}) ─► AgentEvent::ToolCallPending ─► skeleton card
 input_json_delta ×N        ─► throttled Pending{partial_args}
 assistant[tool_use T args] ─► yield asst Msg[ToolRequest T, MARKED] ─► marker seen: skip categorize/dispatch,
                                                                        persist + yield Message  ────────────► card 'loading'
 (child calls /tool_bridge; grant.call executes behind Gate C; result returns to child)
 user[tool_result T]        ─► yield user Msg[ToolResponse T, MARKED] ─► persist + yield Message ────────────► card 'success'/'error', expandable
 text deltas (msg_B)        ─► partial text Msgs, same id            ─► streamed markdown
 result frame               ─► terminal usage                        ─► turn_usage (last snapshot wins)
```

| Axis | Assessment |
| --- | --- |
| Agent-loop invariants | One new branch, in a free function: if the yielded `Message` carries the marker, skip `categorize_tools` (`agent.rs:7178-7205`), push to `messages_to_add`, yield. No `ToolRequest` the loop did not issue is ever dispatched. |
| Persistence & replay | Rows are ordinary paired messages with fresh uuids (the provider message id stays on text rows only, honouring `agent.rs:285-300`); §6.2b's re-split never runs on them because they bypass the dispatch batch. Next-turn `fix_tool_calling` sees request-opens/response-closes and keeps them (`conversation/mod.rs:491-527`). |
| Compaction | Ordinary tool traffic: counted, prunable, capped in flattening at 4,000 chars per result. |
| Transcript flattening | Free: `transcript.rs:53-70` renders the pairs as `[called tool: …]`/`[tool result: …]` in later prompts, so the child remembers its own lookups. |
| Privacy gates | Execution already passed Gate C inside the bridge (`extension_manager.rs:2598-2604` via `bridge.rs:355-364`). The *recording* passes no gate — same as today's flattened text; no new `.call_tool(` site, so the repo-grep count tests hold. |
| Approval flow | Unchanged: `needs_approval` still refused (`bridge.rs:334-339`); the refusal now shows as a red error card naming the tool instead of vanishing. No `ToolCallConfirmation` UI — the stated parity gap. |
| UI parity | Full reuse: `ToolCallWithResponse`, `ToolCallStatusIndicator`, `ToolCallArguments` fed unchanged; pending skeleton replaced by id; status derives exactly as for API providers. |
| Codex built-ins | Same shape with a `child-executed` marker variant (phase 4). |
| Cancellation | Unchanged backstop (`kill_on_drop`); a marked pair interrupted mid-call shows `interrupted` like any API-provider card. |
| Stack cliff | The skip branch and pair validation live in `reply_parts.rs`/new module free functions. |

### Option B — hand-back

The bridge **parks** the child's `tools/call` instead of dispatching it, hands a normal
`ToolRequest` into the provider stream, the loop dispatches it exactly like an API
provider's (inspectors, `ToolCallConfirmation` UI, Gate C sampled by the loop), and the
result is routed back to the parked HTTP responder.

```text
claude child          tool_bridge route        parked-call channel        stream() decoder      Agent::reply loop
 MCP tools/call ────► park responder ────────► send ToolRequest ────────► yield asst Msg[ToolRequest] ─► categorize → inspect →
                                                                                                   (approval UI possible) → dispatch
                                              ◄──────── route result back ◄── loop's ToolResponse ─┘
 ◄── HTTP response with result
```

| Axis | Assessment |
| --- | --- |
| Agent-loop invariants | Clean in principle — the loop dispatches a request it *did* issue — but the stream must now merge child stdout with a bridge-originated channel, and the result must find its way back across tasks to the parked responder: new machinery on both sides of `BridgeGrant`, breaking its "snapshot, not a handle back to the `Agent`" design (`bridge.rs:68-70`). |
| Persistence & replay | Identical to API providers (the §6.2b re-split applies naturally). |
| Approval flow | **The only option that achieves it**: `needs_approval` parks until the user answers in the normal UI. But the child's MCP HTTP client has its own timeout; a minutes-long approval wait (bb recorded 151 s) may kill the child's call anyway — unverified. |
| Privacy gates | Gate C sampled by the loop; the grant's own capability path becomes dead — a second sampling semantics to reconcile. |
| Deadlock surface | The child blocks on HTTP awaiting a result that only arrives after the loop finishes a dispatch batch that only starts after the stream yields — correct in sequence, fragile under parallel `tool_use` blocks and under cancellation. |
| Effort/risk | Highest by far; touches bridge, route, provider, loop and store simultaneously. |

### Option C — side-channel activity events

Extend the advisory channel: keep `PendingToolCall` for the open, add a terminal
"provider tool call resolved" `AgentEvent` (id, name, args, result digest, status) mapped to
a new SSE frame and a new card.

```text
claude child            stream() decoder                Agent loop                GUI
 tool_use start ──────► Pending{id,name} ─────────────► ToolCallPending ────────► skeleton
 tool_result ─────────► ProviderCallResolved{id,...} ─► new AgentEvent ─────────► new resolved card (not a Message)
```

| Axis | Assessment |
| --- | --- |
| Agent-loop invariants | Zero risk — nothing enters `categorize_tools`. |
| Persistence & replay | **Fails parity**: `PendingToolCall`-style events are never persisted; reopening the session shows no tool cards, and the pending-card contract (`chatStreamStore.tsx:985-994`) leaves skeletons stuck unless a same-id `toolRequest` lands — which this option never sends. |
| Transcript flattening | The child forgets its own tool calls next turn (`transcript.rs:72-75` drops non-request/response content). |
| UI parity | Requires a new card and a new terminal-state protocol; the existing components are *not* reused. |
| Effort | Lowest, but it buys live display only, not parity. |

**Recommendation: Option A.** It is the only design that simultaneously (i) reuses the
existing tool-card components and status derivation unchanged, (ii) persists and replays
like an API provider, (iii) feeds the child's own memory through the existing flattening,
(iv) adds no dispatch call site (privacy count tests intact) and no cross-task plumbing,
and (v) leaves the compliance boundary exactly where it is. What it does **not** achieve:
interactive approval of bridged calls (still refused with a visible reason — adopt Option
B's parking later as an isolated extension if wanted), and gate coverage for Codex's
child-executed built-ins (displayed and persisted, but executed in the child's read-only
sandbox outside BioRouter's inspectors — as today, now visible).

## The plan

Effort estimates are engineer-days for someone already familiar with the providers. The
riskiest phase is **phase 3** (the loop-adjacent marker skip and the pair contract).

### Phase 0 — capture fixtures and spike (2 days)

- **Goal:** committed, redacted recordings of real vendor frames, modelled on bb's lanes,
  plus a throwaway `stream()` spike proving the bridge-URL construction rule.
- **Files:** new `crates/biorouter/tests/fixtures/coding_agent/{claude,codex}/<cell>.ndjson`
  (cells: `turn-text`, `turn-thinking`, `turn-tools`, `turn-tool-error`, `auth-failure`,
  `cancel`); a capture harness extending the `#[ignore]`d live tests in
  `crates/biorouter-server/tests/tool_bridge_routes.rs:231-332,519-663` (which already
  drive `claude -p --output-format stream-json --verbose` and parse `assistant`/`result`
  frames).
- **Tests:** none new; the fixtures are the deliverable.
- **Acceptance:** each cell contains a `tool_use`→`tool_result` pair (Claude) or
  `item/started|completed{mcpToolCall}` pair (Codex) for a real bridged BioRouter tool —
  the shape no bb recording contains — plus `stream_event` text/thinking deltas. The Codex
  `mcpToolCall` capture is an explicit **entry gate for phase 3's Codex half**, not a
  formality: a grep over every bb codex lane finds zero `mcpToolCall` frames, and the only
  in-repo sequence is a hand-labelled unit fixture whose item carries just
  `id/server/tool/status` with no `arguments` or `result` (`codex.rs:1170-1174`) — the
  shapes phase 3 needs exist today only as a schema reading.
- **Could go wrong:** an `mcp__biorouter__*` `tool_result` shape differing from the
  Anthropic block inference; Codex `item/completed{mcpToolCall}.result` arriving `null`
  (bb saw `aggregatedOutput:null` on successful closes — the bridge call log backfills).

### Phase 1 — Claude text + thinking streaming (4–5 days)

- **Goal:** `ClaudeCodeProvider::stream()` yields partial text (and thinking) live.
- **Files/functions:** `crates/biorouter/src/providers/claude_code.rs` — override
  `supports_streaming()` and `stream()`; call `command_for(.., "stream-json", ..)` (the
  builder and its argv-invariant tests already parameterise this, `:877-894`) adding
  `--include-partial-messages --verbose`; read `active_bridge_url()`, write the mcp-config
  tempfile and **spawn before returning the stream**, keeping the child, the
  `NamedTempFile` and the stdout reader inside the stream state (`bridge.rs:625-636`).
  New `crates/biorouter/src/providers/coding_agent/claude_stream.rs`: a line router that
  feeds each `stream_event.event` payload — re-prefixed with `data: ` — into
  `formats::anthropic::response_to_streaming_message` (which skips non-`data: ` lines,
  `formats/anthropic.rs:621-626`), **but only the text and thinking events**. The router
  MUST divert every `tool_use` content-block event
  (`content_block_start{tool_use}`, its `input_json_delta`s, and its
  `content_block_stop`) away from the decoder — parked in router state for phase 3's
  hook, a no-op in phase 1 — because the decoder is not a passive text-assembler for
  tool blocks: its §6.2b flush mints a **single batched assistant `Message` full of
  unmarked `ToolRequest`s** (`flush_pending_tool_contents`,
  `formats/anthropic.rs:546-556`, flushed at `message_delta` `:840-846` and again as a
  belt-and-suspenders at `:958-960`) and emits its **own `PendingToolCall`s**
  (`:680-689,:730-740`). An unmarked `ToolRequest` reaching the loop goes through
  `categorize_tools` (`agent.rs:7188`; `categorize_tool_requests` filters on
  `MessageContent::ToolRequest` content only, `reply_parts.rs:357-361`) and is
  dispatched: with the `mcp__biorouter__` prefix intact that is a
  "Tool '…' not found" error row per call (`extension_manager.rs:2530-2536`) and extra
  loop iterations; with the prefix stripped it is a real **second execution** of a call
  the bridge already ran. Left unspecified, this fires from phase 1 onward — before the
  phase-3 marker exists — so the diversion is part of phase 1's contract, not phase 3's.
  The router also **drops `assistant`/`user` frame duplication for
  text/thinking**, and consumes `system/init` (the `apiKeySource` refusal, `:429-434`
  today) and the terminal `result` frame outside the decoder — classification on
  `terminal_reason`/`api_error_status` (an `is_error:true` result can say
  `subtype:"success"`), authoritative usage yielded as the last stream item (the agent
  keeps the last snapshot, `agent.rs:7097-7104`). Thinking blocks are yielded whole at
  `content_block_stop` as the Anthropic decoder already does, **with the signature
  blanked** so the signed-turn persistence branch (`agent.rs:7582-7641`) never engages for
  a provider whose history is flattened to text anyway (`transcript.rs:51`).
  The stream state also carries the **turn ceiling from day one**: a `sleep_until`
  deadline (30 minutes, the same `TURN_TIMEOUT`, `claude_code.rs:115`) raced inside the
  stream's poll, yielding a terminal error and killing the child on expiry. This cannot
  wait for phase 5: today's ceiling is `timeout(TURN_TIMEOUT, child.wait())` inside `run`
  (`:386-401`), which `stream()` never calls, and the loop's cancel check only fires
  between stream items (`agent.rs:7111-7113`) — so a streaming path shipped without its
  own deadline hangs indefinitely on a wedged child, with user cancel as the only
  recovery.
  `crates/biorouter/src/providers/lead_worker.rs` — forward `supports_streaming()`/`stream()`.
- **New types:** none beyond the stream state struct.
- **Tests:** fixture-driven unit tests replaying the phase-0 NDJSON through the decoder,
  asserting the yielded `Message` sequence (stable `message_start` id on every text chunk —
  id-less chunks fragment persistence into one row per delta, `agent.rs:290-294`); a
  fixture test replaying the phase-0 **turn-tools** NDJSON through the phase-1 router
  asserting the yielded sequence contains **zero `ToolRequest`-bearing messages and zero
  decoder-minted `PendingToolCall`s** — the interim contract until phase 3 lands (the
  bridged call still executes and returns to the child; it is just not yet displayed,
  exactly as today); the
  existing captured-frame tests (`claude_code.rs:970-1022`) still pass; a regression test in
  the shape of `the_url_is_visible_inside_the_scope_and_not_outside` (`bridge.rs:720-732`)
  proving the URL is captured at construction; a ceiling test with a wedged fake child
  asserting the in-stream deadline kills it and yields the timeout error; an `--ignored`
  live test asserting first text arrives before the child exits — built **through
  `command_for`**, not a hand-assembled argv, so the real builder finally gets end-to-end
  coverage (the current live tests hand-build theirs, `tool_bridge_routes.rs:250,554`).
- **Acceptance:** text renders progressively in the GUI with correct markdown; usage equals
  the json-mode figure for the same prompt; `assert_subscription_auth` still refuses a
  keyed child; a streamed turn that makes a bridged tool call yields no `ToolRequest`
  rows and no double execution; a wedged child is reaped at the deadline.
- **Could go wrong:** frame drift between 2.1.235 and the recorded 2.1.238 (field-level
  only — the types were cross-checked in the binary); whether plain `-p` with stdin-closed
  behaves identically to the SDK's stream-json-input path (open question; the live test
  answers it).

### Phase 2 — Codex text + reasoning (4 days)

- **Goal:** `CodexProvider::stream()` yields `item/agentMessage/delta` text live; the
  protocol bugs found by this research are fixed in passing.
- **Files/functions:** `crates/biorouter/src/providers/codex.rs` — replace the
  `absorb` fold with a yielding decoder (a free function over `(method, params)` state):
  `item/agentMessage/delta` → partial text `Message` with `id = itemId`;
  `item/completed{agentMessage}` → reconcile, never append (deltas concatenate exactly to
  the final text — verified on all 17 bb recordings — so drop the final rather than emit it);
  carry `phase` so `final_answer` remains the reply even if commentary streams first.
  Fixes: read usage from the **final** `thread/tokenUsage/updated.tokenUsage.total`
  (camelCase) — not `.last`, which is **per model request, not per turn**: a turn with
  tool calls makes several model requests and emits several updates, so `.last` at turn
  end undercounts by every earlier request (the bb turn-tools recording shows it: seq 81
  `total.totalTokens` 19767 = `last`, seq 94 after the second request `total` 39660 while
  `last` is 19893). Because each provider call starts a fresh thread
  (`thread/start` per `turn_on`, `codex.rs:517`), the final snapshot's `total` IS the
  per-call figure. The current `parse_usage(params.get("usage"))` on `turn/completed`
  (`codex.rs:436-439`) is kept as a **fallback**, not deleted: the 0.149.0 recordings
  show `turn/completed` without usage, but the in-repo fixture — documented "as captured
  from a live `codex app-server`" — carries snake_case usage right there
  (`codex.rs:1156-1200`), so with installed 0.147.0 vs recorded 0.149.0, deleting the old
  read could silently zero usage on the installed version (the exact vendor-drift failure
  this plan's risk section warns about). Rule: prefer `tokenUsage/updated` when any
  arrived, fall back to `turn/completed.usage` when present; the captured fixture stays
  as the fallback-path test and the 0.149-shape fixture lands beside it.
  **Keep** the `"turn/failed"` arm (`:440-449`) — it already reads
  `params.error.message` (`:442-444`), and `absorb` deliberately tolerates both the
  app-server and `codex exec` dialects (":424-427: accept both so a version change on
  either surface does not silently drop the answer"), so the arm may be the exec-surface
  or older-version failure path; add `turn/completed{status:"failed", error}` handling
  beside it (the 0.147.0-schema failure shape, unhandled today). In the **separate**
  `"error"` notification arm (`:453-465`, which reads `params.message`), read the message
  from both `params.message` and `params.error.message` — no recording contains a
  `method:"error"` frame, so its wire shape is unproven either way and a swap would
  trade one guess for another. Answer approvals with `"decline"` not the invalid
  `"denied"` (`:403-415`); keep the `turn/start` result's `turn.id` (`:550-560` discards
  it) for phase 5's `turn/interrupt`. The stream state carries the same **in-stream
  30-minute deadline as phase 1** — today's ceiling wraps `run_turn`'s join
  (`codex.rs:550-558`), which the stream path bypasses — as part of this phase's
  acceptance, not phase 5's. Optionally send `summary:"auto"` on `turn/start`
  behind an env flag and map `item/reasoning/summaryTextDelta`/`textDelta` to Thinking
  content — unobserved in any recording, so treat as experimental.
- **Tests:** extend the scripted Python fake app-server (`codex.rs:943-976`) to emit
  `item/started` + N×`item/agentMessage/delta` + `item/completed` + `thread/tokenUsage/updated`
  before `turn/completed`, asserting yield order and no duplicated final text; a fake-server
  test emitting **two** `thread/tokenUsage/updated` frames asserting the turn's usage
  equals the second `total`, not the second `last`; a fallback test where only
  `turn/completed{usage}` carries usage (the captured-fixture shape) asserting it still
  lands; fixture replays from phase 0; the captured-sequence test (`:1156-1200`) **kept**
  as the fallback-path test and extended to assert what is *yielded*, not just what is
  kept; a ceiling test with a wedged fake asserting the in-stream deadline fires.
- **Acceptance:** Codex text streams in the GUI; usage numbers are non-zero and match the
  final `thread/tokenUsage/updated.tokenUsage.total` (with the `turn/completed.usage`
  fallback intact); a failed turn surfaces its error whether it arrives as `turn/failed`
  or `turn/completed{status:"failed"}`; a wedged app-server is reaped at the deadline.
- **Could go wrong:** whether 0.147.0 tolerates the old `"denied"` reply was never
  testable — fixing it is safe either way; reasoning deltas may simply never fire on
  subscription models (ship text first, reasoning as a follow-up).

### Phase 3 — tool-call parity for bridged calls (mirror; 5–6 days)

- **Goal:** every bridged call appears as a live card the moment it is made, resolves to
  success/failure, and expands to args and result — through the unchanged existing UI.
- **Files/functions:**
  - `crates/biorouter/src/conversation/message.rs` — a `provider_executed` marker as a
    reserved key in the existing `ToolRequest`/`ToolResponse` provider `metadata`
    (`message.rs:60,65-76,97`); serialized, so persisted rows carry it (no new
    `MessageContent` variant enters SQLite). **Not** a `MessageProvenance` variant —
    that type is BR-71's security-purposed cross-session stamp with its own consumers
    (`message.rs:588-610`, `conversation/mod.rs:599`,
    `subagent_handler.rs:451,1037,2059`) and lossy unknown-kind degradation; see Option A
    above. If message-level provenance is ever wanted here it is a separate field, and
    its change comes with an audit of every provenance consumer. Because
    `ToolRequest`/`ToolResponse` are `ToSchema` types surfaced through the server's
    OpenAPI spec, this phase ends with `just generate-openapi && cd ui/desktop && npm run
    generate-api` (CLAUDE.md's standing rule) so the GUI reads the marker through a
    regenerated client rather than a stale one.
  - `crates/biorouter/src/providers/coding_agent/claude_stream.rs` — consume the
    `tool_use` content-block events that phase 1's router diverted: emit
    `PendingToolCall{id: toolu_…, name (prefix-stripped), partial_args}` from
    `content_block_start{tool_use}`/`input_json_delta` at the existing 200 ms / 200 char
    cadence (`formats/anthropic.rs:17-19`); on the `assistant` frame's `tool_use`, yield
    the marked assistant `Message[ToolRequest]` (fresh uuid message id); on the `user`
    frame's `tool_result`, yield the marked user `Message[ToolResponse]` with
    `is_error` mapped to the error side.
  - `codex.rs` decoder — same pairs from `item/started|completed{mcpToolCall}` keyed by the
    codex item id. **Entry gate:** this half does not start until phase 0 has captured a
    real bridged `mcpToolCall` sequence (see phase 0 — the shapes are schema-read only
    today); the Claude half proceeds independently. Fallback if the capture shows
    `item/started` never firing or `item/completed` carrying no `arguments`/`result`:
    build the cards entirely from the bridge call log keyed by arrival order, accepting
    degraded pending timing.
  - `crates/biorouter/src/providers/coding_agent/bridge.rs` — a per-turn call log on the
    grant/lease (request id, tool name, post-rewrite args, result digest, error, duration),
    read by the decoder to enrich args (hooks may rewrite inputs, BR-19) and to backfill a
    `null` Codex result. No event emission — the log is pull-only.
  - `crates/biorouter/src/agents/reply_parts.rs` — a free function
    `take_provider_executed(response) -> Option<…>` used at the top of the loop's
    per-item handling: marked messages are pushed to `messages_to_add` and yielded as
    `AgentEvent::Message`, skipping `categorize_tools` entirely (the stack-cliff rule:
    nothing inline in `reply_internal`, `agent.rs:855-862`).
- **Tests:** loop-level unit tests proving (a) a marked request is never dispatched (no
  `Tool not found`, no second execution — the double-execution failure mode is concrete:
  `agent.rs:7178-7205` would otherwise dispatch and `extension_manager.rs:2524-2534` would
  error), (b) the persisted transcript holds the pair in order and survives
  `fix_tool_calling`, (c) `transcript::flatten` renders the pair for the next turn;
  fixture-driven decoder tests asserting pending → request → response id equality; an
  extension of `tests/streaming_pending_tool_calls.rs` for the marked path.
- **Historical acceptance (superseded by the subscription credential-isolation allowlist):** in
  the GUI, a bridged `developer__shell` call shows a skeleton within a
  token of the child starting the block, becomes a `loading` card, resolves with the green
  or red indicator, and expands to the exact arguments and output; reloading the session
  shows the same cards; the next turn's child prompt contains the flattened pair; the
  OpenAPI spec and generated TS client are regenerated in the same change (CI's spec
  check would fail otherwise).
- **Could go wrong:** ordering — the response can only be yielded after the child echoes
  the result, which for Claude is immediate but for a cancelled call never arrives (the
  turn-end backfill and the `interrupted` status cover it); marker forgery — a hostile
  vendor stream could mark arbitrary requests, but a marked request is *never executed*,
  so the worst case is a false card, not a false dispatch.

### Phase 4 — Codex child-executed built-ins (2–3 days)

- **Goal:** `commandExecution`/`fileChange` items (read-only sandbox) and calls to the
  user's own `~/.codex/config.toml` MCP servers surface as clearly-labelled
  **child-executed** cards.
- **Files:** the codex decoder — same mirror pairs from `item/started|completed`, marker
  variant `child_executed`, args = `{command, cwd}` / change list, result =
  `aggregatedOutput` + `exitCode` (repairing from the bridge log is impossible here — these
  never touched the bridge); status map `inProgress→loading`, `completed→success`,
  `failed→error`, `declined→interrupted`. A small UI affordance (a label on the card, e.g.
  "executed by Codex, read-only sandbox") is the one visual change in the whole plan.
- **Tests:** fixture replay of the bb-shape `commandExecution` frames; vitest for the label.
- **Acceptance:** a Codex turn that runs `sed` shows the command card with its output;
  nothing suggests BioRouter executed or gated it.
- **Could go wrong:** flooding — Codex can run many tiny commands; cap or coalesce is a
  judgment call left to review.

### Phase 5 — cancellation and cooperative interrupt (2–3 days)

- **Goal:** the streaming path cancels as crisply as the blocking one. The 30-minute
  ceiling itself is **not** this phase's deliverable: it ships inside each provider's
  `stream()` in phases 1 and 2 (the blocking-path timeouts — `claude_code.rs:386-401`,
  `codex.rs:550-558` — wrap awaits the stream path never reaches, and the loop's
  between-items cancel check (`agent.rs:7111-7113`) only fires when an item arrives, so
  streaming shipped without an in-stream deadline would hang indefinitely on a wedged
  child; phases 1–2 must not create that interim).
- **Files:** both providers — `kill_on_drop(true)` remains the backstop for hard drops
  (`state.rs:441` → `workspace/turn.rs:751-755` drop-on-cancel; the source-text test in
  `coding_agent/mod.rs:199-226` keeps pinning it). The loop's own between-items token check
  (`agent.rs:7111-7113`) now actually fires mid-turn, ending the stream early; Codex gains
  an optional cooperative `turn/interrupt {threadId, turnId}` (the id kept in phase 2) so
  already-streamed text settles like bb's interrupt recordings show; the deadline paths
  landed in phases 1–2 are audited for idempotence against `kill_on_drop`.
- **Tests:** fake-server tests asserting a cancelled stream reaps the child and yields what
  it had (bb's "settles a partially streamed assistant message when interrupted" is the
  model); the ceiling tests themselves live in phases 1–2.
- **Acceptance:** Stop mid-stream leaves the partial text and `interrupted` tool cards in
  the transcript; no orphaned `claude`/`codex` processes.
- **Could go wrong:** double-kill races between the deadline path and `kill_on_drop` —
  harmless but noisy; assert idempotence.

### Phase 6 — UI verification and parity gate (3 days)

- **Goal:** verified rendering, and the parity test of the section below wired into CI.
- **Files:** vitest additions beside `chatStreamStore` and `ToolCallWithResponse` tests;
  the artifact-harness/browser sweep for streamed markdown (half-open fences during
  streaming re-parse per frame in `ui/desktop/src/components/MarkdownContent.tsx:467-469`
  — identical to the Anthropic path today, so parity is by construction; bb's
  settled-prefix/live-tail split is noted as optional polish, not scope).
- **CLI:** the GUI is not the only consumer of these frames — the CLI's live half
  (`session send`/`watch`/`attach`, `crates/biorouter-cli/src/commands/`) reads the same
  HTTP + SSE stream. One acceptance item watches a streamed coding-agent turn in the
  terminal and confirms partial text renders sanely and the mirrored tool-pair
  user-messages and `ToolCallPending` frames degrade gracefully (no garbled duplicates,
  no stuck pending lines). Parity is otherwise defined GUI-first by decision, but the CLI
  must not regress.
- **Acceptance:** the vitest parity suite passes; a human check in the dev GUI
  (`BIOROUTER_NO_HMR=1`) confirms skeleton → loading → success/error → expand on both
  providers; the CLI watch check above passes.

### Phase 7 — documentation and stale statements (1–2 days)

- Fix the self-contradiction in [performance and limits](performance-and-limits.md): its
  body (`:95-105`) correctly says the bridge does not block streaming, while its own
  Related-documentation footer (`:131-132`) still blames "the grant lifetime that currently
  prevents streaming" — the footer is stale and must go; the body's "There is no streaming
  yet" section is rewritten to describe the shipped path.
- Fix [child-agent isolation](child-agent-isolation.md) line 34, which describes a
  "streaming path" (`stream-json`) that does not exist in production today.
- Fix the stale "once the bridge lands" comments: `codex.rs:27-34`,
  `coding_agent/mod.rs:52`, `claude_code.rs:616-617` — the bridge landed.
- Rewrite the comment at `agent.rs:7046-7048`, which becomes false the day `stream()`
  ships, to state the construction-time rule from `bridge.rs:625-636` instead.
- Update [how it works](how-it-works.md) and [the tool bridge](tool-bridge.md) with the
  mirror record. (The folder `README.md` index row for this page is **not** deferred to
  this phase: per `docs/organization.md`'s discipline the index row ships with the
  document itself — the row, with Status: Proposed, lands in the same change that adds
  this file, so a reader entering the folder through its README finds the plan during
  phases 0–6, not after them.)

## Parity test strategy

The assertion to make executable: **the same recorded scenario yields the same `Message`
sequence and the same rendered cards through the Anthropic provider path and the
`claude_code` path.**

- **Rust.** A new test binary (e.g. `crates/biorouter/tests/coding_agent_streaming_parity.rs`)
  loads two fixtures of one logical scenario — the Anthropic SSE lane and the Claude
  stream-json lane recorded in phase 0 — runs each through its decoder, and compares the
  yielded sequences after normalization: ids interned by first appearance, timestamps
  zeroed, provider-metadata markers erased on the coding-agent side. Asserted: same ordered
  content-kind sequence (text… / pending… / ToolRequest / ToolResponse / usage), same
  request↔response id pairing, same error classification for the failure cells, and pinned
  per-cell counts (`messages`, `pending`, `unhandled` — with `unhandled` only allowed to go
  down), exactly bb's `row-counts.json` discipline. The Codex lane joins the same table:
  raw frames differ, normalized `Message` rows must not.
- **Vitest.** The recorded scenarios are exported once more as SSE `MessageEvent` frame
  logs — **by running each scenario through the real reply route against a fake provider**
  (a small integration harness), not by hand-mapping decoder output to frames. The
  mapping is not mechanical: `routes/reply.rs` carries the #59 persisted-ordering seam
  (no `MessagesPersisted` may precede the `Message` frame carrying an id it publishes;
  the coalescer flush that guards it is pinned by the route's own tests,
  `crates/biorouter-server/src/routes/reply.rs:828-868`), and a hand-exported log would
  bypass exactly the framing whose edge cases can break the GUI — a test that passes
  while the feature is broken. At minimum, one Rust test asserts the exported log
  byte-matches what the real route emits for one cell. A store
  test then feeds each log into `chatStreamStore` and asserts the identical final store shape —
  message list, `pendingToolCalls` empty, per-card derived status — for the Anthropic and
  claude_code logs of the same cell; a component test renders `ToolCallWithResponse` from
  both and snapshots the status indicator and expanded args/result identically. jsdom
  cannot catch the Prism/Tailwind class of rendering bug, so the phase-6 browser sweep
  stays mandatory.
- **Live.** The `#[ignore]`d suite in `tool_bridge_routes.rs` gains one end-to-end case per
  provider asserting: first text `Message` arrives before child exit; the bridged call's
  card pair is present and marked; usage lands on the terminal item.

## Risks, invariants and open questions

**Invariants this plan must not break** (each is enforced somewhere):

- A partial/pending tool call never reaches `categorize_tools` or dispatch
  (`tests/streaming_pending_tool_calls.rs`; `agent.rs:7124-7130`).
- On the coding-agent path, the reused Anthropic decoder never receives `tool_use`
  content-block events — its §6.2b flush would mint an unmarked dispatchable
  `ToolRequest` batch (`formats/anthropic.rs:546-556,840-846,958-960`); phase 1's
  zero-`ToolRequest` fixture test pins this.
- The bridge URL is read at construction time, never from a poll (`bridge.rs:625-636`).
- Every streamed chunk carries a stable message id, or persistence fragments
  (`agent.rs:290-294`).
- `kill_on_drop(true)` precedes `.spawn()` in both providers (source-text test,
  `coding_agent/mod.rs:199-226`).
- New stream-path logic lives in free functions, never inline in `reply_internal`
  (`agent.rs:855-862`, issue #87 — the stack cliff).
- No new call sites for `raise_privacy`, `floor` or `.call_tool(` (repo-grep assertions);
  the mirror design adds none.
- The MessageStream contract: partial text, complete tool calls, third slot advisory-only
  (`base.rs:1063-1070,1097-1100`).

**Risks.** Vendor drift is the standing one: the recordings are 2.1.238/0.149.0 against
installed 2.1.235/0.147.0, and Codex ships multiple times a day — the fixture pins plus the
live `--ignored` tests are the alarm. The marker skip is the sharpest edge: a bug there
either re-executes tools (caught by the phase-3 unit test) or silently drops them from
dispatch for API providers (impossible by construction — only coding-agent decoders mint
the marker, but the test asserts an unmarked request still dispatches). The
`message_has_user_visible_progress` counter ignores `ToolRequest` content
(`agent.rs:204-210`), so a turn that is all tool cards registers no visible progress for
the loop-guard machinery — benign today, worth a glance in review.

**Open questions**, carried from the research:

- Does plain `claude -p` (stdin closed) with `--include-partial-messages` emit frames
  identically to the SDK's `--input-format stream-json` path the recordings came from?
  (Phase 0/1 live test answers it.)
- Does Codex 0.147.0 accept the invalid `{"decision":"denied"}` today, and does
  `summary:"auto"` on `turn/start` ever produce reasoning deltas on subscription models?
- Exact `tool_use`/`tool_result` payload for an `mcp__biorouter__*` tool over `--mcp-config`
  HTTP — inferred from Anthropic block shapes, never yet recorded (phase 0 records it).
- Whether `MessageContent::Thinking` should stay GUI-invisible (parity with the Anthropic
  provider today) or the GUI grows a renderer — out of this plan's scope either way.
- Whether a later phase adopts Option B's parking for the approval flow, and what the
  child-side MCP client timeout does to a minutes-long approval wait.

## Related documentation

- [Coding-agent providers](README.md) — the folder index and the reading order.
- [How the coding-agent providers work](how-it-works.md) — the blocking mechanism this
  plan replaces, including the flattening and usage arithmetic.
- [Installing and signing in](installing-and-signing-in.md) — the setup states the
  streaming path inherits unchanged.
- [The tool bridge](tool-bridge.md) — the relay whose calls the mirror makes visible, and
  the current description of the marker, the two execution kinds and the remaining gaps.
- [What the child agent may not do](child-agent-isolation.md) — the isolation flags the
  streaming invocation must carry verbatim.
- [Compliance: vendor terms, BAA and PHI](compliance.md) — why both providers stay
  `ProviderTier::Public` regardless of streaming.
- [Performance, limits and known gaps](performance-and-limits.md) — what the shipped
  streaming path costs, and the limits it does not remove.
