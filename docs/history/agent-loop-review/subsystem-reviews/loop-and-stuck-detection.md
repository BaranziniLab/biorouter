# Loop detection, repetition and stuck states — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It documents the five layered mechanisms that bound runaway agent behaviour — the `RepetitionInspector`, the 100-iteration turn cap, cancellation, provider retry, and `/goal` stall detection — and records ten gaps.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, whose findings were then implemented. Gaps #1 and #2 (a single hard deny, and the model being falsely told the user declined) were fixed by BR-29; near-duplicate and A/B/A/B oscillation detection by BR-30, with repeated-failing-result detection by BR-31; #5 and #8 (unused `call_counts`, an iteration-only cap) by BR-34; #6 (no wall-clock budget) by BR-35 (`agents/budget.rs`); and #9 (stall detection confined to `/goal`) by BR-32 (`agents/stall.rs`).
> **Audience:** developers working on the agent loop, loop safety, or cancellation.

This review is scoped to how the agent avoids infinite loops, repetitive tool calls, runaway turns, and stuck states. Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); `REP-001` is the runtime finding id the repetition inspector emits; and the numbered items under "Gaps and weaknesses" are what sibling reviews cite as `loop-detection.md #N` (the file's former name).

Contents:

- [Overview](#overview) — the five mechanisms, and one reply in a diagram
- [Infinite loops and repetitive identical tool calls](#infinite-loops-and-repetitive-identical-tool-calls)
- [Max-turns and max-tool-calls caps](#max-turns-and-max-tool-calls-caps)
- [How cancellation works mid-turn](#how-cancellation-works-mid-turn)
- [Local-minimum detection beyond exact duplicates](#local-minimum-detection-beyond-exact-duplicates)
- [How the provider retry loops interact](#how-the-provider-retry-loops-interact)
- [Notable design choices](#notable-design-choices-worth-keeping) and [Gaps and weaknesses](#gaps-and-weaknesses)

## Scope and files reviewed

- `crates/biorouter/src/tool_monitor.rs` (165 lines)
- `crates/biorouter/src/tool_inspection.rs` (the trait + result-application glue)
- `crates/biorouter/src/agents/retry.rs` (workflow-level RetryManager)
- `crates/biorouter/src/providers/retry.rs` (provider transient-error retry)
- `crates/biorouter/src/agents/agent.rs` (the main reply loop — turn cap,
  cancellation, truncation continuation)
- `crates/biorouter/src/execution/manager.rs` (AgentManager / session cache)
- Supporting: `crates/biorouter/src/agents/goal.rs`,
  `crates/biorouter/src/agents/mcp_client.rs`,
  `crates/biorouter/src/utils.rs`,
  `crates/biorouter-server/src/routes/reply.rs`.

> **Note.** The review recorded no commit or branch, and line numbers in the citations below have drifted since; treat them as pointers to the right function, not exact locations.

## Overview

There is **no single "loop detector."** Loop/stuck defenses are spread across
five independent mechanisms, layered from cheap-and-local to
expensive-and-global:

1. **Exact-duplicate tool-call guard** — `RepetitionInspector` in
   `tool_monitor.rs`, run as one of the `ToolInspector`s before every tool
   batch. Denies the Nth consecutive byte-identical call (N > 3).
2. **Per-reply turn cap** — `DEFAULT_MAX_TURNS = 100` iterations of the main
   agent loop in `agent.rs`, a soft stop that asks the user to continue.
3. **Cooperative cancellation** — a `CancellationToken` polled at loop
   boundaries and threaded into MCP tool calls, tripped mainly by client
   disconnect (the "stop" button closing the SSE stream).
4. **Provider transient-error retry** — bounded exponential backoff in
   `providers/retry.rs` (3 attempts generic, 8 for HTTP 429).
5. **Goal-stall give-up** — only for explicit `/goal` sessions: fuzzy
   similarity of judge feedback across attempts (`goal.rs`).

Data flow of one reply (simplified):

```text
reply(user_msg, session_config, cancel_token)               agent.rs:1240
  └─ reply_internal → async_stream loop                      agent.rs:1525
       loop {                                                 agent.rs:1556
         if is_token_cancelled(cancel_token) break            agent.rs:1557
         turns_taken += 1; if turns_taken > max_turns break   agent.rs:1571,1576
         drain_soft_interrupts()  (mid-turn user injects)     agent.rs:1589
         stream = provider.stream(...)                        agent.rs:1603
         while chunk in stream {                              agent.rs:1628
            if is_token_cancelled break                       agent.rs:1629
            on tool_requests:
              inspection = tool_inspection_manager
                              .inspect_tools(...)              agent.rs:1723
                └─ RepetitionInspector::inspect → Deny(REP-001) tool_monitor.rs:107
              apply_inspection_results_to_permissions → denied tool_inspection.rs:181
              dispatch approved tools (cancel_token passed)    agent.rs:836/1757
                └─ mcp await_response select! on cancel        mcp_client.rs:357
         }
         if no_tools_called:
            finish_reason=="length"? auto-continue (≤12)       agent.rs:2053
            else final_output / retry_logic / stop-hook        agent.rs:2072-2232
       }
```

Provider retries (`providers/retry.rs`) sit *inside* `provider.stream(...)`;
the workflow `RetryManager` (`agents/retry.rs`) sits at the very end, resetting
history and re-running when `retry_config` success-checks fail.

## Review questions answered

### Infinite loops and repetitive identical tool calls

The only general-purpose repetition guard is `RepetitionInspector`
(`tool_monitor.rs`). It is registered as a `ToolInspector` at
`agent.rs:355-357` with threshold `DEFAULT_MAX_REPETITIONS = 3`
(`agent.rs:70`).

**What it tracks / how "identical" is defined.** A call is reduced to
`InternalToolCall { name, parameters }` and two calls match on **exact tool
name plus byte-exact JSON argument equality**:

```rust
// tool_monitor.rs:18-20
fn matches(&self, other: &InternalToolCall) -> bool {
    self.name == other.name && self.parameters == other.parameters
}
```

**Two divergent implementations live in this one struct**, and this matters:

- `check_tool_call` (`tool_monitor.rs:59-88`) is the imperative, stateful
  version that mutates `last_call` / `repeat_count` and returns `false` when
  `repeat_count > max_repetitions`. **It is only invoked from tests**
  (`crates/biorouter/tests/repetition_inspector_tests.rs:25-47`). The
  production loop never calls it.
- `inspect` (`tool_monitor.rs:107-164`) is the `ToolInspector` trait method
  that production actually runs (via `inspect_tools`, `agent.rs:1723`). Because
  it takes `&self`, it cannot mutate the struct; instead it **re-derives the
  streak from scratch every batch** by walking the full `messages` history and
  then the new `tool_requests` (`tool_monitor.rs:119-161`), counting only
  **consecutive** identical calls and resetting to 1 on any different call
  (`tool_monitor.rs:128-143`).

**Threshold / trigger.** The comparison is strict `>` (`tool_monitor.rs:145`),
so three consecutive identical calls are allowed and the **fourth** is denied.
On trigger it emits:

```rust
// tool_monitor.rs:151-159
InspectionResult {
    action: InspectionAction::Deny,
    reason: format!("Tool '{}' has exceeded maximum repetitions", tool_name),
    confidence: 1.0,
    inspector_name: "repetition",
    finding_id: Some("REP-001"),
}
```

**What happens on trigger.** `apply_inspection_results_to_permissions`
(`tool_inspection.rs:181-261`) moves the request out of `approved` /
`needs_approval` into `denied` (`tool_inspection.rs:217-236`). Denied tools are
then handled by `handle_denied_tools` (`agent.rs:748-781`), which writes a
synthetic tool response with `is_error: true` and the text `DECLINED_RESPONSE`:

> "The user has declined to run this tool. DO NOT attempt to call this tool
> again. If there are no alternative methods to proceed, clearly explain the
> situation and STOP." (`agents/tool_execution.rs:38-40`)

Critically, the repetition-specific reason is **swallowed**: only *hook*
denials get their reason appended to the model-visible text
(`agent.rs:757-766`). So when the repetition inspector fires, the model is told
"the user declined" — which is false, and gives the model no signal that it was
looping. The tool simply does not execute; the turn continues.

### Max-turns and max-tool-calls caps

Yes for turns, **no dedicated cap for tool calls**.

`DEFAULT_MAX_TURNS = 100` (`agent.rs:69`). Per reply it is resolved as
`session_config.max_turns` → env `BIOROUTER_MAX_TURNS` → default
(`agent.rs:1547-1550`). `SessionConfig.max_turns` is `Option<u32>`
(`agents/types.rs:91`); the interactive `/reply` route hard-codes
`max_turns: None` (`reply.rs:297`), so ordinary chat always uses 100.

`turns_taken` increments once per loop iteration (`agent.rs:1571`). At
`turns_taken > max_turns` the loop yields a **soft** message and breaks:

```rust
// agent.rs:1576-1583
if turns_taken > max_turns {
    yield AgentEvent::Message(Message::assistant().with_text(format!(
        "I've reached my action limit for this turn ({max_turns} actions without
         user input), so I'm stopping here ... Would you like me to continue?
         (raise the cap with `max_turns` / `BIOROUTER_MAX_TURNS`.)")));
    break;
}
```

It is a graceful stop that invites continuation, not a hard error.

**No max-tool-calls cap.** The cap counts loop *iterations* (provider
round-trips). A single iteration can request and run many tools in parallel
(the combined `stream::select_all`, `agent.rs:1792-1843`), so "100 actions" is
a loose bound on real work and cost. The `call_counts` HashMap in
`RepetitionInspector` (`tool_monitor.rs:46`) tracks per-tool totals but is
**never read for any decision** — there is no "this tool has run 200 times"
guard.

Adjacent bounds that also cap loop-like behavior:
- `MAX_TRUNCATION_CONTINUATIONS = 12` (`agent.rs:76`) — auto-continues of a
  `finish_reason=="length"` truncated response, reset to 0 whenever a tool runs
  (`agent.rs:2053-2118`).
- `compaction_attempts >= 2` (`agent.rs:1967`) — context-overflow compaction
  retries before giving up.
- `STOP_HOOK_BLOCK_CAP = 5` (`hooks/mod.rs:44`) — how many times a Stop hook can
  block completion before finishing anyway (`agent.rs:2160-2178`).
- Goal system: `GOAL_MAX_ITERATIONS = 20`, `GOAL_STALL_LIMIT = 3`
  (`goal.rs:53-55`).

### How cancellation works mid-turn

Cancellation is **cooperative**, via `tokio_util::sync::CancellationToken`
threaded from `reply` → `reply_internal` (`agent.rs:1244,1525`) and into tool
dispatch.

**How it is tripped.** For interactive chat the dominant trigger is *client
disconnect*. `stream_event` in `reply.rs` cancels when the SSE `tx.send` fails,
i.e. the browser closed the stream (the stop button aborts the fetch):

```rust
// reply.rs:195-198
if tx.send(format!("data: {}\n\n", json)).await.is_err() {
    tracing::info!("client hung up");
    cancel_token.cancel();
}
```

The server task also selects on `task_cancel.cancelled()` (`reply.rs:347-350`)
and drops the reply stream when it breaks. The Agent Drafter WebSocket path adds
an explicit `{"type":"cancel"}` frame that calls `cancel.cancel()`
(`routes/apps.rs:856`) plus `ui_bridge.cancel_all()`.

**Where it is observed.** `is_token_cancelled` (`utils.rs:44-48`) is polled at
three loop boundaries: the top of the main loop (`agent.rs:1557`), before each
provider stream chunk (`agent.rs:1629`), and while draining the combined tool
result stream (`agent.rs:1798-1800`).

**In-flight tool calls.** The token is passed into `dispatch_tool_call`
(`agent.rs:836-960`) and reaches the MCP client's `await_response`, which
`select!`s the response against a timeout and the token, and on cancel sends an
MCP `cancel` notification to the server before returning `Cancelled`:

```rust
// mcp_client.rs:365-376
tokio::select! {
    result = receiver => { ... }
    _ = tokio::time::sleep(timeout) => { send_cancel_message(...,"timed out"); Err(Timeout) }
    _ = cancel_token.cancelled() => { send_cancel_message(...,"operation cancelled"); Err(Cancelled) }
}
```

So a remote MCP tool is told to abort. The turn's usage is still recorded once
on stream end "whether the stream finished, was cancelled, or errored"
(`agent.rs:2032-2037`).

**Gap:** cancellation is checked *between* awaited items, never inside a
blocking tool body. A built-in tool that ignores the token (e.g. a synchronous
computation) keeps running until it returns; the loop only notices at the next
boundary. There is no forced tokio task abort — dropping the stream relies on
`Drop`/`kill_on_drop` for subprocesses to actually stop work. Also note the
soft-interrupt path (`/interrupt` → `queue_soft_interrupt` →
`drain_soft_interrupts`, `reply.rs:494-505`, `agent.rs:1589-1594`) is a
deliberate *non-cancelling* alternative that injects a user message at a safe
boundary.

### Local-minimum detection beyond exact duplicates

Is anything watching for the same failing approach tried repeatedly?

Mostly **no**, and this is the biggest honest gap.

`RepetitionInspector` catches only **exact consecutive duplicates** (identical
name + byte-identical JSON). It is trivially defeated by:
- changing any argument by one character (a different retry of the same failing
  command counts as "new"),
- oscillation `A, B, A, B, ...` (the streak resets on every switch,
  `tool_monitor.rs:130-133`),
- semantically identical but textually different calls.

It also does **not** look at tool *results* at all — repeated identical error
messages, "no such file" over and over, or a command that keeps failing the same
way are invisible to it. There is no no-progress detector (e.g. "no file changed
/ no new information in N turns"), no embedding/semantic similarity, and no
result-hash tracking.

The **one** place with fuzzier stall detection is the goal system
(`goal.rs`), and it only applies to explicit `/goal` sessions. When a goal is
active, each Stop-hook block increments `iterations`, and consecutive judge
feedback strings are compared with `reason_similarity` (Jaccard overlap of word
sets, `goal.rs:121-133`); `stall_count` increments when similarity ≥
`GOAL_STALL_SIMILARITY = 0.5` and the goal gives up when
`stall_count >= GOAL_STALL_LIMIT (3)` or `iterations >= GOAL_MAX_ITERATIONS
(20)` (`goal.rs:301-320`). That is genuine progress-stall detection — but for
ordinary chat it is absent. For a normal conversation the only guardrails
against a stuck loop are: (a) the exact-duplicate inspector, and (b) the 100-turn
hard cap.

### How the provider retry loops interact

Could the retries themselves loop?

Provider retries are **self-bounded and cannot loop infinitely on their own.**
`providers/retry.rs` retries only transient errors
(`RateLimitExceeded | ServerError | RequestFailed`, `should_retry`
`retry.rs:117-124`) with exponential backoff + jitter, capped at
`DEFAULT_MAX_RETRIES = 3` (`retry.rs:34`), or `RATE_LIMIT_MAX_RETRIES = 8`
for HTTP 429 (`retry.rs:44-54`). Both `retry_operation` (`retry.rs:126-173`)
and `ProviderRetry::with_retry` (`retry.rs:182-233`) terminate after the budget
and return `Err`.

Three distinct "retry" layers exist and do **not** feed each other into a loop:
1. Provider transient retry (`providers/retry.rs`) — inside a single
   `provider.stream(...)`.
2. Agent loop error handling — on a non-retryable/exhausted provider error the
   loop yields "Ran into this error… Please retry if you think this is
   transient" and **breaks** (`agent.rs:2020-2027`); on `ContextLengthExceeded`
   it compacts, bounded by `compaction_attempts >= 2` (`agent.rs:1964-2018`).
   Neither re-enters provider retry automatically, so no tight loop forms.
3. Workflow `RetryManager` (`agents/retry.rs`) — after a turn ends, if
   `retry_config` success-checks fail and `attempts < max_retries`
   (`retry.rs:130-155`), it resets the whole conversation to `initial_messages`
   and re-runs. Bounded by `max_retries`.

**Compounding risk, not infinite loop.** Retries *multiply*: each of up to 100
agent turns can itself do up to 3 (or 8) provider retries, and 429 backoff can
be ~8 × up to 30 s ≈ 2 minutes *per provider call* inside a turn. There is **no
global wall-clock or token budget per reply** — only the iteration count — so a
throttled session can legitimately run a very long time without tripping any
loop guard. BioRouter does surface rate-limit state to the scheduler via
`RATE_LIMITED_UNTIL_MS` (`retry.rs:14-32`) so *background* jobs back off, which
is a nice touch, but it does not bound the interactive reply.

**Dead-code caveat.** `RetryManager` can hold a `repetition_inspector` and
`reset_attempts` would reset it (`agents/retry.rs:63-83`), but
`with_repetition_inspector` is **never called** — the agent constructs
`RetryManager::new()` (`agent.rs:258`). So that reset is a permanent no-op, and
the `RepetitionInspector` instance registered in the inspection manager is a
*different* object whose imperative state is never touched in production anyway.

## Notable design choices (worth keeping)

- **Soft turn cap that asks the user** rather than hard-failing, and is
  user-raisable via `max_turns` / `BIOROUTER_MAX_TURNS` (`agent.rs:1576-1583`).
- **Cancellation that notifies the remote tool** — the MCP `cancel`
  notification (`mcp_client.rs:373-375`) lets servers abort real work, not just
  detach a future.
- **Soft interrupt as a first-class alternative to cancel** — injecting a
  mid-turn user message at a safe boundary avoids discarding in-flight work and
  a full context re-send (`reply.rs:494-505`, `agent.rs:1589-1594`).
- **Deeper, separate retry budget for 429s** plus scheduler-visible rate-limit
  state so background load stops piling onto a throttled key
  (`retry.rs:44-54,14-32`).
- **Truncation continuation with its own streak cap that resets on real
  progress** (`agent.rs:2053-2118`) — distinguishes "cut off by length limit"
  from "chose to stop."
- **Goal give-up-on-stall** — fuzzy feedback similarity plus an iteration cap is
  a genuinely good local-minimum pattern (`goal.rs:301-320`); it just needs to
  reach general chat.
- Inspectors are pluggable and fail-open per inspector (a crashing inspector is
  logged and skipped, `tool_inspection.rs:108-116`) — robust, though see the gap
  about swallowed reasons.

## Gaps and weaknesses

These ten items fed the improvement phase. They are what other documents in this
review cite as `loop-detection.md #N`; the numbering below is that scheme and is stable.

1. **Exact-duplicate only — trivially defeated.** `matches` requires byte-exact
   JSON (`tool_monitor.rs:18-20`) and only counts *consecutive* calls. A
   one-character arg change, an `A/B/A/B` oscillation, or the same failing
   command with a tweaked flag all bypass it. State-of-the-art coding agents
   detect repeated *failing outcomes* and no-progress loops; BioRouter does not.

2. **The block reason is hidden from the model.** On a repetition denial the
   model receives generic `DECLINED_RESPONSE` ("the user has declined…"), not
   the true "exceeded maximum repetitions" (`agent.rs:757-766` only forwards
   *hook* reasons). This is actively misleading: the model may abandon a tool it
   legitimately needs, or hallucinate that the user refused. The REP-001 reason
   should be surfaced with guidance to change approach.

3. **Two implementations, tested path ≠ production path.** `check_tool_call`
   (stateful) is what the unit tests exercise
   (`tests/repetition_inspector_tests.rs`), but production runs `inspect`
   (stateless). The struct's `last_call` / `repeat_count` / `call_counts` /
   `reset()` are effectively dead in production. High risk that a future fix or
   tuning lands in the untested-in-prod path. Consolidate to one implementation.

4. **`inspect` recomputes over full history every batch** (`tool_monitor.rs:
   119-134`) — O(messages) per tool batch, quadratic across a long session.
   Minor now, but it also means the "3 strikes" window is *global consecutive*
   and silently resets whenever any different call is interleaved, weakening it
   further.

5. **`call_counts` is tracked but never used.** No absolute per-tool ceiling
   (`tool_monitor.rs:46,61-65`). A tool called hundreds of times with
   ever-changing args never trips anything except the 100-turn cap.

6. **No global time or token budget per reply.** Only iteration count bounds a
   turn; provider 429 backoff (~2 min/call) compounds inside it. A pathological
   or throttled session can run far longer than a user expects with no
   wall-clock guard.

7. **Cancellation is cooperative and boundary-only.** A tool that ignores the
   token blocks cancel until it returns; there is no hard task abort, and cancel
   checks never occur inside a tool body (`agent.rs:1557,1629,1798`). Long
   synchronous built-in tools are effectively uncancellable mid-execution.

8. **`max_turns` counts loop iterations, not tool calls.** With parallel tool
   fan-out per iteration, the 100 "actions" bound is loose relative to real
   cost/side-effects.

9. **Local-minimum detection is gated behind `/goal`.** The good stall logic in
   `goal.rs` never runs for ordinary chat, which is where most stuck loops
   actually happen.

10. **Latent dead-code coupling.** The workflow `RetryManager`'s
    repetition-inspector reset is never wired (`agent.rs:258` uses
    `RetryManager::new()`), so anyone reasoning about "history reset also resets
    repetition state" is wrong — a subtle trap.

## Net assessment

The subsystem is defensive enough to prevent a truly
unbounded hang (the 100-turn cap and self-bounded provider retries guarantee
termination), but its *loop-quality* detection is well behind modern coding
agents: it catches only literal identical-call spam, hides the reason from the
model, tests a code path production never runs, and has no notion of "same
failing approach" or "no progress" outside the opt-in goal system.

## Related documentation

- [Self-verification, output validation and done-ness](self-verification-and-doneness.md) — the sibling review that covers the `/goal` judge and no-progress detection from the verification side; the two overlap on stall detection.
- [Guardrails, security and the permission system](guardrails-and-permissions.md) — the inspector chain the `RepetitionInspector` runs inside, and where a denial's reason gets swallowed.
- [Safety and guardrails compared with other agents](../competitive-comparison/safety-and-guardrails.md) — how these loop bounds measure against eight other coding agents.
- [Core agent loop and tool dispatch](core-loop-and-tool-dispatch.md) — the loop whose iterations `max_turns` counts.
- [Wave 2 loop detection report](../../agent-loop-campaign/wave-reports/wave-2-loop-detection.md) — what was actually built in response to these gaps.
