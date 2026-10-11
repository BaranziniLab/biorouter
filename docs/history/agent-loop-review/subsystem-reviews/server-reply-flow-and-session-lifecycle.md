# Server reply flow and session lifecycle — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It traces a GUI message through the `biorouterd` daemon — the SSE `/reply` route, session creation and resume, cancellation, the action-required approval pause, and auth and concurrency — and records eleven gaps.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, whose findings were then implemented. Gap #1 (no server-side single-turn-per-session lock, which this review calls "the single most important gap") was fixed by BR-33, gap #2 (confirmation waits with no TTL) by BR-36, and the orphaned `/interrupt` route and the missing abort endpoint by BR-58 and BR-61-class work.
> **Audience:** developers working on `biorouter-server` routes or the desktop chat stream.

Despite the subsystem often being described as "HTTP/WS", this review's own finding is that **there is no WebSocket in the chat reply path** — the agent loop streams to the GUI over Server-Sent Events. The only WebSocket in the tree is the per-app Agent Drafter socket, which is out of scope here. Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); the numbered items under "Gaps and weaknesses" are what the [review README](../README.md) and sibling reviews cite as `server-flow.md gap #N` (the file's former name). The answer sections below are deliberately unnumbered so that citation form is unambiguous.

## Scope and files reviewed

The subsystem is the `biorouter-server` reply, session, agent and action-required routes, and how the
Electron GUI drives them. All paths are repository-relative.

Backend (Rust):

- `crates/biorouter-server/src/routes/reply.rs`
- `crates/biorouter-server/src/routes/agent.rs`
- `crates/biorouter-server/src/routes/session.rs`
- `crates/biorouter-server/src/routes/action_required.rs`
- `crates/biorouter-server/src/routes/mod.rs`
- `crates/biorouter-server/src/state.rs`, `auth.rs`, `commands/agent.rs`
- `crates/biorouter/src/execution/manager.rs`, `agents/agent.rs`, `agents/tool_execution.rs`

Frontend (TypeScript):

- `ui/desktop/src/hooks/chatStreamStore.tsx`, `hooks/useChatStream.ts`, `components/ToolCallConfirmation.tsx`

> **Note.** The review recorded no commit or branch; line numbers have drifted since and several of the `ui/desktop/src` components have moved. Treat every citation as a pointer to the right function, not an exact location.

## Overview

The daemon (`biorouterd`) is an Axum HTTP server bound to loopback (`127.0.0.1`,
`configuration.rs:70/84`). Every route is wrapped by one auth middleware, a CORS layer that only
admits loopback origins, and gzip compression (`commands/agent.rs:53-62`). There is **no WebSocket**
in the chat reply path — the agent loop streams to the GUI over **Server-Sent Events (SSE)**. (The
only WS in the tree is the per-app Agent Drafter socket in `routes/apps.rs`, out of scope here.)

Text data-flow for one user turn:

```text
GUI chatStreamStore.submitPreparedMessage
  └─ POST /reply  {session_id, user_message}            (SSE, AbortController.signal)
       reply() spawns a tokio task:
         state.get_agent(session_id)  → AgentManager LRU (get_or_create_agent)
         session_manager.get_session  → load prior Conversation
         agent.reply(user_message, SessionConfig, cancel_token)  → BoxStream<AgentEvent>
         select! loop:  agent event | 500ms Ping heartbeat | cancel
           each event → `data: {json}\n\n` written to mpsc → SSE body
  ← stream of MessageEvent {Message|UpdateConversation|ModelChange|Notification|Ping|Error|Finish}

Permission ask (mid-turn):
  agent yields a `toolConfirmationRequest` Message → SSE → GUI shows ToolCallConfirmation card
  agent's inner loop BLOCKS on confirmation_rx.recv()
  GUI POST /action-required/tool-confirmation {id, action, sessionId}
       → agent.handle_confirmation → confirmation_tx.send → unblocks loop → tool dispatched
```

The reply route never touches the socket directly; it owns an `mpsc::channel(100)` whose receiver is
adapted into an SSE body (`reply.rs:86-125`, `247-248`), and a `CancellationToken` shared with the
agent (`reply.rs:249`, `321-326`).

## Review questions answered

### Full request path — GUI message, route, streamed events, tool-call round-trip

**Submit.** The GUI builds a `Message` and calls the generated `reply()` client, which issues
`client.sse.post('/reply', …)` (`ui/desktop/src/api/sdk.gen.ts:447`). The body carries only
`session_id` and `user_message` (`chatStreamStore.tsx:536-544`); `conversation_so_far`,
`workflow_name`, `workflow_version` are optional and the GUI omits them. An `AbortController.signal`
is attached and `sseMaxRetryAttempts: 1`.

**Route.** `POST /reply` → `reply()` (`reply.rs:214-485`). It is registered with a 50 MB body limit
(`reply.rs:509-512`). The handler immediately returns an `SseResponse` (`reply.rs:484`) and does all
work in a spawned task (`reply.rs:257`), so the HTTP response headers flush before the model runs.

Inside the task: `state.get_agent(session_id)` resolves a shared `Arc<Agent>` from the
`AgentManager` LRU (`reply.rs:262`, `state.rs:106-111`, `manager.rs:84-116`). The session row is
loaded (`reply.rs:278`) to build a `SessionConfig { id, schedule_id, max_turns: None, retry_config:
None }` (`reply.rs:294-299`). Conversation history comes from the session unless
`conversation_so_far` was supplied, in which case it **replaces** the persisted conversation
(`reply.rs:301-318`). The new user message is appended (`reply.rs:319`) and `agent.reply(...)` returns
a `BoxStream<AgentEvent>` (`reply.rs:321-342`, `agent.rs:1240`).

**Streaming.** A `tokio::select!` loop (`reply.rs:345-406`) multiplexes three sources: the
`cancel_token`, a 500 ms `Ping` heartbeat, and `timeout(500ms, stream.next())`. Each `AgentEvent` is
mapped to a `MessageEvent` variant (`reply.rs:128-156`) and serialized as `data: {json}\n\n`
(`stream_event`, `reply.rs:183-199`). Event kinds: `Message` (with per-event `TokenState`),
`UpdateConversation` (from `HistoryReplaced`, e.g. after compaction), `ModelChange`, `Notification`
(MCP), `Ping`, `Error`, and a terminal `Finish` (`reply.rs:356-482`). Token counts are re-fetched per
event via a **lightweight** `get_token_counts` query rather than a full session load
(`reply.rs:158-181`).

**GUI consumption.** `streamFromResponse` (`chatStreamStore.tsx:450-515`) `for await`s the SSE
iterator and switches on `event.type`, mapping to `ChatState` (`Streaming`/`Thinking`/`Compacting`/
`WaitingForUserInput`). A `Message` containing a `toolConfirmationRequest` or an `elicitation`
`actionRequired` flips state to `WaitingForUserInput` (`chatStreamStore.tsx:466-475`). `Finish`/`Error`
call `finishCurrentStream`.

**Tool-call confirmation round-trip.** When the agent needs approval it *yields a normal assistant
`Message`* carrying an `ActionRequired`/tool-confirmation content
(`tool_execution.rs:161-169`), then the inner loop **blocks** on
`self.confirmation_rx.lock().await; while let Some((req_id, confirmation)) = rx.recv().await`
(`tool_execution.rs:171-172`). The GUI renders `ToolCallConfirmation` and, on click, POSTs
`/action-required/tool-confirmation` with `{id, action, sessionId, principalType:'Tool'}`
(`ToolCallConfirmation.tsx:102-110`). The route maps the action string to a `Permission`
(`always_allow`/`allow_once`/`deny`) and calls `agent.handle_confirmation` (`action_required.rs:34-56`),
which sends `(request_id, confirmation)` down `confirmation_tx` (`agent.rs:1228-1236`). The blocked
loop matches `req_id`, dispatches the tool if allowed (recording `AlwaysAllow`/`AlwaysDeny` into the
permission manager), or writes a `DECLINED_RESPONSE` tool result (`tool_execution.rs:173-229`). The
resulting tool response streams back over the *same* SSE connection.

### Sessions — created, resumed, listed, and where context is injected

**Create.** `POST /agent/start` (`agent.rs:216-364`). It resolves an optional workflow
(deeplink/id/inline), validates it, then `session_manager.create_session(working_dir, "New Session",
SessionType::User)` (`agent.rs:262-275`). The name is always the literal placeholder `"New Session"`;
"named or not" is tracked by `user_set_name` (`agent.rs:257-262`). Enabled extensions are resolved and
persisted into `extension_data` (`agent.rs:281-303`), the workflow is saved
(`agent.rs:305-318`), and extension loading is **eagerly** kicked off in a background task stored in
`AppState.extension_loading_tasks` (`agent.rs:332-361`, `state.rs:46-53`) so `/agent/resume` can reuse
the result.

**Resume.** `POST /agent/resume` (`agent.rs:377-437`). Loads the session; if
`load_model_and_extensions` is true it gets the agent, `restore_provider_from_session`, and either
consumes the background extension-loading task or loads extensions synchronously
(`agent.rs:393-431`). The GUI calls this on session load (`chatStreamStore.tsx:319-345`), then fires
`updateFromSession` (`chatStreamStore.tsx:355-361`).

**List / read.** `GET /sessions` → `list_sessions` (`session.rs:136-146`); `GET
/sessions/{id}` → `get_session` with `is_valid_session_id` guard (`session.rs:165-179`,
`115-121`). Extra reads: `/sessions/insights`, `/sessions/activity` (server clamps `days` to 1..=371,
`session.rs:204-243`), `/sessions/{id}/extensions`. Mutations: name, `user_workflow_values`, delete,
export, import, `edit_message` (fork/truncate), and `diverge` (branch a full copy)
(`session.rs:245-670`).

**Context injection — route level.** `/reply` injects only conversation state and `SessionConfig`; it
does **not** set a system prompt (`reply.rs:294-319`). The **desktop system prompt** is injected by
*other* routes: `/agent/update_from_session` (`agent.rs:449-498`) and `/agent/restart` /
`/agent/update_working_dir` (`agent.rs:712-768`) render `desktop_prompt.md` plus any workflow prompt
and call `agent.extend_system_prompt`. So the GUI must call `updateFromSession` after resume to give
the agent its desktop persona — this is a two-request handshake, not part of `/reply`.

**Context injection — agent level.** `agent.reply` (`agent.rs:1240`) runs hooks (`SessionStart` once,
then `UserPromptSubmit`, which can *deny* the prompt or inject context — `agent.rs:1275-1332`),
handles slash commands (`agent.rs:1344`), and `reply_internal`/`prepare_reply_context`
(`agent.rs:1519-1534`) assembles the system prompt, tools, and toolshim tools. Per-iteration it injects
MOIM context (`super::moim::inject_moim`, `agent.rs:1596-1601`) and drains soft interrupts
(`agent.rs:1589-1594`).

### Cancellation from the UI

Cancellation is driven by **closing the SSE connection**, not a dedicated cancel endpoint.
`stopStreaming` bumps `activeStreamId` and calls `abortController.abort()`
(`chatStreamStore.tsx:668-673`). Aborting the fetch closes the HTTP body; the server's next
`tx.send(...)` fails, and `stream_event` reacts by calling `cancel_token.cancel()`
(`reply.rs:195-198`). The reply `select!` loop observes `task_cancel.cancelled()` and breaks
(`reply.rs:347-350`); the agent's inner loop polls `is_token_cancelled(&cancel_token)` at the top of
each turn and inside the provider stream (`agent.rs:1557`, `1629`, `1798`). The dropped `BoxStream`
also drops any pending future (including a permission-wait `recv`).

Note `POST /agent/stop` exists (`agent.rs:695-710`) but only removes the agent from the manager LRU
(`manager.rs:118-125`); because the in-flight reply task holds its own `Arc<Agent>` clone, `stop` does
**not** cancel a running turn. There is no `session_id`-addressed "cancel this turn" HTTP route — the
client must own the SSE socket to stop generation.

### Action-required pause and resume

Two distinct mechanisms:

- **Tool permission asks** pause the loop by blocking on the `confirmation_rx` mpsc channel inside
  `handle_approval_tool_requests` (`tool_execution.rs:171-172`); resume is the
  `/action-required/tool-confirmation` POST → `confirmation_tx.send` path traced in the request path above. Before prompting, a
  `PermissionRequest` hook can auto-allow/deny without any user prompt (`tool_execution.rs:67-131`),
  and a `Notification` hook fires when a prompt is shown (`tool_execution.rs:133-148`).

- **MCP elicitations** use a separate `ActionRequiredManager`. When the user answers, the GUI sends a
  **new** `/reply` whose `user_message` contains `ActionRequiredData::ElicitationResponse`; `reply`
  detects it up front, calls `ActionRequiredManager::global().submit_response(...)`, persists the
  message, and returns an **empty** stream (`agent.rs:1248-1269`). The paused elicitation resumes
  inside whatever turn was awaiting it, rather than through the confirmation channel.

### Rate limiting, concurrency control and auth

**Auth.** One middleware, `check_token` (`auth.rs:52-99`), applied to the whole router
(`commands/agent.rs:54-57`). It requires header `X-Secret-Key` compared in **constant time**
(`secret_matches`, `auth.rs:14-24`). Exemptions: `/status`, `/mcp-ui-proxy`, `/mcp-app-proxy`, and
`GET /apps/*` (browser-opened apps that can't send the header; the app WS additionally validates
`Origin`) (`auth.rs:57-72`). The key is `BIOROUTER_SERVER__SECRET_KEY` or a random 16-byte hex per
process (`commands/agent.rs:35-42`).

**Rate limiting.** A per-IP failed-attempt throttle: max 20 failures per 60 s window, keyed on the
real peer address (via `into_make_service_with_connect_info`, not the spoofable `x-forwarded-for`)
(`auth.rs:26-49`, `77-85`, `commands/agent.rs:76-82`). This only counts **auth failures** — an
authenticated client faces no request-rate cap.

**CORS.** Only loopback origins are allowed (`is_local_origin`, `mod.rs:9-24`;
`commands/agent.rs:46-51`), parsed (not prefix-matched) to reject `http://127.0.0.1:8080.evil.com`.

**Concurrency.** There is **no server-side one-turn-per-session guard**. `AgentManager` hands back a
shared `Arc<Agent>` (`manager.rs:84-116`); `/reply` spawns a task with no "is a turn already running?"
check (`reply.rs:257`). Two concurrent `/reply` calls for the same `session_id` would share one
`Agent`, one `confirmation_rx`, and one `soft_interrupts` vec — interleaving their turns. Serialization
is enforced **only client-side**: `canSubmitMessage` refuses to submit while an un-aborted
`abortController` exists (`chatStreamStore.tsx:559-565`). The only global cap is
`BIOROUTER_MAX_ACTIVE_AGENTS` bounding the LRU of *live agents* (`manager.rs:57-63`), not concurrent
turns.

## Notable design choices (worth keeping)

- **Handler returns SSE immediately, does work in a spawned task** (`reply.rs:257`, `484`): response
  headers flush instantly; client hang-up is detected via `tx.send` failure and converted to
  cooperative cancellation (`reply.rs:195-198`).
- **Cancellation by socket close** is simple and leak-free: no cancel token to leak, dropping the
  stream frees pending futures including a parked permission `recv`.
- **Soft interrupt** (`/interrupt` → `queue_soft_interrupt`, `reply.rs:498-505`, `agent.rs:297-305`)
  injects a mid-turn user message at a safe loop boundary (`agent.rs:1589-1594`) instead of
  cancel-and-resend — a genuinely nicer UX than most agents, which discard in-flight work.
- **Constant-time secret compare + connect-info-keyed throttle** (`auth.rs`) avoid two real classes of
  bug (timing side-channel, `x-forwarded-for` spoofing).
- **Lightweight per-event token query** (`get_token_counts`, `reply.rs:158-181`) avoids a full session
  + `COUNT(*)` on the hot streaming path.
- **Eager background extension loading** on `/agent/start`, consumed by `/agent/resume`
  (`agent.rs:332-361`, `393-431`), hides MCP startup latency.
- **500 ms `Ping` heartbeat** keeps the SSE connection and proxies alive during long tool calls
  (`reply.rs:351-353`).
- **Input validation**: `is_valid_session_id` (`session.rs:115-121`), `days` clamp, name length cap.

## Gaps and weaknesses

These eleven items fed the improvement phase. They are what other documents in this
review cite as `server-flow.md gap #N`; the numbering below is that scheme and is stable.

1. **No server-enforced single-turn-per-session.** The only guard is the GUI's `abortController` check.
   A second `/reply` (from a second window, the CLI, a retry, or a raced click) can start a concurrent
   turn on the same `Arc<Agent>`, sharing `confirmation_rx`/`soft_interrupts` and interleaving output.
   State-of-the-art agents hold a per-session turn lock/queue server-side. This is the single most
   important gap. (`reply.rs:257`, `manager.rs:84-116`.)
2. **Confirmation channel is not request-scoped.** `confirmation_rx` is one mpsc per agent
   (`agent.rs:152-153`). Concurrent turns, or a stale/duplicate `/action-required` POST, can deliver a
   confirmation to the wrong pending request; the loop just drops non-matching `req_id`s
   (`tool_execution.rs:172-173`) with no timeout, so a lost confirmation blocks the turn **forever**
   (until the client disconnects). No TTL, no "prompt expired" path.
3. **Permission wait ignores the cancel token.** `rx.recv().await` (`tool_execution.rs:171-172`) is not
   in a `select!` with `cancel_token`; a mid-prompt cancel only works because the client closes the
   socket and the stream is dropped. A programmatic cancel (`/agent/stop`) would not unblock it.
4. **`/agent/stop` does not stop a turn** (`agent.rs:695-710`) — misleadingly named; it evicts the
   agent from the LRU while the reply task keeps its own `Arc`. There is no addressable
   "abort the running turn" endpoint independent of owning the SSE socket.
5. **`conversation_so_far` lets the client overwrite server history.** `/reply` will
   `replace_conversation` with a client-supplied array (`reply.rs:301-318`) via
   `Conversation::new_unvalidated` — trusting unvalidated client state as the source of truth for a
   turn. Loopback + secret mitigates, but it is a large, unvalidated trust surface.
6. **`/interrupt` (soft interrupt) is orphaned.** It has no `#[utoipa::path]`, is absent from
   `openapi.json`, is not in the generated TS client, and the GUI never calls it (verified: no
   `interrupt` symbol in `api/sdk.gen.ts`). A nice feature is effectively dead for desktop users.
7. **Rate limiting only covers auth failures.** An authenticated client has no request/turn quota; a
   buggy loop or a shared-key multi-client setup can spawn unbounded concurrent `/reply` tasks
   (`auth.rs:32-41`).
8. **No idempotency / retry safety on `/reply`.** With `sseMaxRetryAttempts: 1`
   (`chatStreamStore.tsx:543`), an SSE reconnect re-POSTs and would start a *second* turn (appending the
   user message again) rather than resuming the first — there is no turn id or resume token.
9. **Per-event DB round-trips.** `get_token_state` runs once **per streamed event**
   (`reply.rs:363`, `369`, `472`); on a chatty stream this is many small SQLite queries where a cached
   counter would do.
10. **Fragile client-side conversation merge.** `pushMessage` reconciles streamed deltas with
    string-prefix/`JSON.stringify` heuristics (`chatStreamStore.tsx:105-144`); brittle versus a
    server-authoritative message-id/patch protocol.
11. **Global mutable env at startup.** `BIOROUTER_APP_BASE_URL` is set via `std::env::set_var`
    (`commands/agent.rs:69`) — process-global mutation used as ambient config.

## Related documentation

- [Core agent loop and tool dispatch](core-loop-and-tool-dispatch.md) — what `agent.reply(...)` does once this route hands off to it.
- [Long-running tasks, background processes and scheduling](long-running-tasks-and-scheduling.md) — the `ActionRequiredManager` elicitation path this review contrasts with tool-permission confirmation.
- [Execution and verification compared with other agents](../competitive-comparison/execution-and-verification.md) — how this reply pipeline measures against eight other coding agents.
- [Sessions guide](../../../getting-started/managing-sessions.md) — the current, living reference for session creation, resume and export.
- [Wave 2 server cancellation report](../../agent-loop-campaign/wave-reports/wave-2-server-cancellation.md) — what was actually built in response to gaps #1 to #4.
