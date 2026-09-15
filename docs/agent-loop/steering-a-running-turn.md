# Steering a running turn

> **What this is.** The contract behind "Add this message to the current turn": what happens to a steer between the click and the model reading it, the promises the daemon and the renderer each keep, what every way a turn can end does with a steer it accepted, and how the composer's activity indicator stays honest while a steer waits.
> **Status:** Current. Describes `fix/steer-always-lands` — `agents/agent.rs` (the soft-interrupt queue and its wakes), `agents/loop_phase.rs`, `agents/workspace_extension.rs` (`workspace_watch`), `providers/claude_code.rs` (the stream-json pump), `biorouter-server`'s `routes/reply.rs` (`POST /interrupt`), `workspace/turn.rs` (the detached runner), `turn_stream.rs` (the orphan reaper) and `state.rs` (continuation leases), and the desktop's `hooks/chatStreamStore.tsx`, `utils/trailingActivity.ts` and `components/ChatInput.tsx`.
> **Audience:** anyone changing the reply loop, the `/interrupt` route, the turn runner, a live-steering provider, or the composer's queue and activity indicator.

## The promise

A steer the daemon accepted (it answered `202`) always reaches the agent loop, whatever the loop is doing — a tool call, a model stream, an approval card, delegation — or, when the turn ends before the model can read it, it is stored and shown as **not answered**. A steer never ends a turn early, a turn is never stopped for having been steered, and the composer's spinner turns only while the daemon's turn is genuinely alive.

## Where a steer can be waiting, and what it waits for

The loop records what it is awaiting in a phase on the `Agent` (never in the reply generator, whose poll frame sits within a few percent of the stack limit during delegation). `GET /sessions/running` serves it as `turn_phases`, the `/interrupt` route logs `steer_accepted … phase=… phase_age_ms=…`, and the agent logs `steer_consumed latency_ms=… via=loop_top|live_ack|carried_over|exit_requeue`.

| Phase | What the steer does |
|---|---|
| `prologue` — extension wait, hooks, auto-compaction | Accepted: the runner opens the queue before it waits on extensions or calls `Agent::reply`. Consumed at the loop's first step. |
| `provider_open` — waiting for the first byte | A restart-steering provider (Versa, OpenAI-compatible streams) drops the pending request and reissues it with the steer. Only a steer the person typed does this; another chat's injection waits for the next boundary. |
| `streaming` | Restart-steering: the stream is dropped and reissued, tool-call skeletons it announced are retracted (`ToolCallsRetracted`), and the model is told its partial answer was interrupted. Live-steering (Claude Code, Codex): the steer is handed to the child and its acknowledgement is a wake, never an await. |
| `gating`, `tool_batch` | The tool is never killed. The renderer is told once (`SteerWaiting { reason: "tool", tool_name }`) and the steer lands at the next boundary. `workspace_watch` is the exception: a read-only wait, it returns early on the person's steer. |
| `approval_wait` | A steer never answers a card — free text is not a permission decision. The renderer shows "Your message will be added after you answer the card", with no clock. |
| `exit_gates` — done gate, self-critique, Stop hooks | Accepted. The queue closes only at the commit point immediately before the loop really ends; a steer found there keeps the turn going. |
| `supervision_wait` — a forced exit collecting children | Accepted. See the exit table. |
| `settling` — after the commit point | Refused with `409 {"reason":"turn_closing"}`; clients send the text as a new turn at the terminal frame. |

## What each ending does with an accepted steer

| Ending | Steer |
|---|---|
| Normal completion | Drained at the commit point; the turn continues and answers it. |
| `max_turns`, `max_tool_calls`, stall stop | A steer the person typed restarts the count ("actions without user input") and is answered. One typed after the loop broke, while it waits for children, ends that wait and the runner continues the same turn with it — same turn id, one `TurnFinished`. |
| Spend cap, provider abort, stream error, structured final output | Stored with `steerOutcome: "unanswered"`, after any prose the reply had streamed. |
| Stop | Stored unanswered, between the partial reply and the "Stopped." notice. |
| Orphan reap | Never while a card is parked; an accepted steer restarts the clock. A reap finishes as `reason: "orphaned"`. |

A steer is removed from the queue only after its row is stored, so a Stop landing mid-write leaves it for the runner's settle rather than dropping it.

## The renderer's side

- **Ownership.** `ChatStreamController.steer` owns a steer until the daemon answers: a network error, a 5xx or no answer within 8 s is retried with backoff under one idempotency key (the daemon answers a repeated key `202` without queuing twice); a `409`, `400` or `403` is a refusal, and a refused queued message goes back to its own queue slot.
- **Retirement.** The chip is retired by the echo's message **id**, not by position, so a compaction mid-turn cannot strand it, and an echo marked unanswered never retires it.
- **Honest spin.** The indicator keys on `chatState` plus a live owner: a driving socket, an observer, an attach or submit in flight, a Stop, or a reconcile. Every frame — the daemon's 500 ms `Ping` included — is a heartbeat. Ten seconds of silence closes the socket and asks `/agent/resume`: the same turn is rejoined with backoff for as long as the daemon names it, another turn is attached, and no turn ends the spin without an error card. A running state with no owner for ten seconds is logged and reconciled the same way.
- **Refused sends.** A `/reply` refused with `409` before a turn was admitted gives the words back to the composer and attaches to whatever turn holds the chat.

## Connection budget

Chromium allows six connections per host across every window. Observers are capped at two (`MAX_LIVE_OBSERVER_STREAMS`) because the renderer also parks two long-polls and the running `/reply`, which leaves one slot for a steer or a Stop; a unit test pins that sum. Each observer iteration closes its own socket when its stream ends, and CORS preflights are cached for ten minutes.

## Tests

- `cargo test -p biorouter --test steer_always_lands` — provider-open race, restart retraction and the interrupted-answer note, live acknowledgement as a wake, Stop during the acknowledgement wait, `SteerWaiting` during a long tool, and the forced-exit continuation.
- `cargo test -p biorouter --test soft_interrupt_agent_loop`, `--test turn_abort_tests`, and `--test subagent_delegation` (the stack margin).
- `cargo test -p biorouter-server --lib -- a_steer_in_the_turn_prologue an_interrupt_409_names_its_reason a_retried_steer_with_the_same_key a_turn_parked_on_a_card an_unclaimed_continuation_lease a_stream_error_settles`.
- `cd ui/desktop && npx vitest run src/hooks/chatStreamStore.steerLands.test.tsx src/utils/trailingActivity.test.ts`.

## Related documentation

- [Workspace control](workspace-control.md) — steering from the terminal, and what a daemon without a user-action key refuses.
- [Turn cancellation and process reaping](turn-cancellation-and-process-reaping.md) — the Stop half of turn control.
- [Subagents](subagents.md) — steering a delegated child from its tab.
- [Decisions behind `biorouter serve`](../deployment/serve-decisions.md) — SD-11, why a keyless daemon refuses steers.
