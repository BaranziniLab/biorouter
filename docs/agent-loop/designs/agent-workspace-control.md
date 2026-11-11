# Agent workspace control and glass-box subagents (BR-71)

> **What this is.** The feature request and design for a `workspace` control surface that
> lets the BioRouter agent drive the desktop GUI and the daemon as first-class MCP tools —
> open/close/switch chat tabs and windows, read any conversation's transcript and tool-call
> history, inject prompts into other conversations, change a session's tool/extension/KB
> set at runtime — and, as its flagship embodiment, turns today's opaque subagents into
> **glass-box subagents**: every spawned subagent appears as a live, ordinary chat tab
> that a human can watch, talk to, and intervene in, exactly as the parent agent can.
> **Status:** Current — **all four slices are implemented.** The task-by-task record of
> how, and the gates each one had to pass, is
> [`br71-execution-plan.md`](br71-execution-plan.md); that plan is the change log, this
> document stays the design of record. Slice 1 shipped the session model, the event
> spine, the one turn runner and
> the headless `workspace_*` tools; Slice 2 shipped the GUI bridge (`GET /ui/workspace`),
> `workspace_open`, the observer-backed tab, provenance chips, the focus-etiquette
> setting and the chatrecall suggestion — verified end to end against the **dev build**,
> where `workspace_list` reports `gui_attached: true` and a background tab under `gui`.
> The **packaged** app has not been run end to end; its one materially different input,
> the renderer origin, was measured separately (see §4.3): a `file://` document in
> Electron 39.8.10 / Chromium 142 sends `Origin: file://`, not `null`, which is exactly
> what `check_workspace_ws_auth` admits. Slice 3 shipped the glass-box subagents
> themselves — child sessions on the detached runner, the spawn-context record, the
> auto-opened badged tab and its header, the human steer path, `human_intervened` in the
> parent's result, the Stop control, History grouping by parent, and the CLI half
> (`session list --subagents`, `session attach`, `session cancel`). Slice 4 shipped the
> polish: the tuned instruction block and the tool-routing table, `consult` unified onto
> this spine (§8.2), the CLI-parity gate that fails the build when a workspace capability
> has no CLI counterpart, and the user docs —
> [`docs/extensions/built-in/workspace.md`](../../extensions/built-in/workspace.md) and
> the glass-box half of [`docs/agent-loop/subagents.md`](../subagents.md), which also
> carries the `subagent_status` migration table (decision 23). §8's five open questions
> are closed in place below.
>
> **Still outstanding, and not closed by the slice count:** Task 40b's undiagnosed daemon
> abort (below), tracked in the plan doc. **Task 44's release gates were RUN on
> 2026-08-01 and all pass** — 82 test binaries / 3976 passed with only the two recorded
> environmental failure families, 202 desktop test files / 1837 tests, both harness tiers
> at exit 0 including all 25 live assertions, and the CLI end-to-end chain against a
> daemon with no desktop app attached; the per-gate numbers are in the plan doc under
> Task 44. That is **not** a sign-off: Task 40b is still open, and Task 44's own text
> forbids signing off while it is. §8.4's resync cost, which that task was to record here,
> is **2-4 ms** to the first `UpdateConversation` frame for a 1-message session.
>
> **What Slice 3's gate actually measured** (read this before quoting the line above).
> The flagship chain is verified end to end **headlessly**, against a running daemon, by
> `scripts/workspace/glassbox-harness.mjs`: with `BIOROUTER_HARNESS_LIVE=1` a real parent
> spawns a real child and all 25 assertions pass — `open_tab` + `annotate_tab` naming the
> parent, the child observer stream, the spawn-context record, `POST /interrupt` into the
> **running** child returning 202 (the turn lease, not a 409), the steer landing stamped
> `user_direct`, `cancelled:true` **with** a turn id, the tab-composer `/reply` stamped
> `user_direct`, `"human_intervened":true` in the parent's transcript, and decision 21's
> two-tier surface on that same never-configured parent (`workspace__subagent` offered,
> `workspace__workspace_list` withheld). The CLI half was
> driven by hand against the same daemon with no desktop app attached: children nest
> under their parent with distinct labels and a `● live` marker, `attach` renders the
> conversation so far as a transcript before following it live, a typed line returns
> `[steered turn …]` in the same turn, `cancel` reports the turn id and is idempotent,
> and ctrl-C detaches without cancelling.
>
> **What it did NOT measure**, in full, so the paragraph above cannot be quoted as more
> than it is: **the live GUI acceptance pass in the desktop app has not been run.** Four
> of Task 40's bullets rest on unit and component tests only — the auto-opened tab with
> its badge and header (decision 24), the four-tab fan-out cap (decision 26), the
> refusal of nesting from inside a child's own tab (decision 25), and the
> elicitation-in-a-child-tab check (reconciliation #8). Decision 21 used to be a fifth
> and is not any more: it is asserted by the live tier above on every run.
>
> **One open defect found by that gate, not yet diagnosed — tracked as Task 40b** in
> [`br71-execution-plan.md`](br71-execution-plan.md), where it blocks the release gate.
> A subagent that calls an
> Auto Visualiser tool aborts the daemon with `fatal runtime error: stack overflow` in a
> `tokio-runtime-worker` — reproduced 3/3 on fresh daemons, with and without
> `BIOROUTER_AUTOVIS_CDN=1`, and **not** reproducible for the same tool in an ordinary
> session. It is not the cancel path (it reproduces with no cancel). The blocking
> delegation path awaits the child's whole agent loop inside the parent's
> `dispatch_tool_call`, so the child's frames sit on the parent's stack; that nesting
> predates this campaign, so whether Slice 3 merely pushed an already-marginal stack over
> the edge is unresolved and needs a baseline build to settle.
>
> BR-71 is a post-campaign proposal, numbered as the next free identifier after the
> campaign's BR-1…BR-70; it does not appear in the campaign master list.
> **Audience:** developers working on the agent loop, `biorouter-server`, and the desktop GUI.

---

## 1. Motivation

Two problems, one mechanism.

**The agent cannot see or shape its own workspace.** BioRouter's daemon already runs many
concurrent agents (an LRU registry of up to 100, keyed by session id —
`crates/biorouter/src/execution/manager.rs:19-24`), and the GUI already renders them as
tabs and split panes. But the agent itself has no tool surface over any of it. It cannot
start a sibling conversation, look at what another conversation concluded, hand work to a
tab the user can watch, or adjust which extensions a conversation is allowed to use. Every
one of those operations exists as an HTTP route or a reducer action today (§3) — none is
reachable from a tool call.

**Subagents are black boxes.** When the main agent delegates via the `subagent` tool
(`crates/biorouter/src/agents/subagent_tool.rs:27`), the child runs a full agent loop with
its own session — but:

- Its event stream is deliberately not user-visible
  (`crates/biorouter/src/agents/subagent_handler.rs:277-278`); the parent LLM receives only
  the final summary (`subagent_result.rs:48-65`), and the human receives nothing at all.
- Its session is persisted but **excluded from the browsable session list** —
  `list_session_summaries` filters `WHERE session_type IN ('user','scheduled')`
  (`crates/biorouter/src/session/session_manager.rs:3255`), so a `sub_agent` transcript
  cannot be opened in the GUI even after the fact.
- The human cannot see what prompt/context the child was started with, what tools and
  knowledge bases it was granted, what it is doing, or inject course corrections. The only
  affordance is discovery-and-kill via the active-work registry
  (`crates/biorouter-mcp/src/active_work.rs`, HTTP at `routes/active_work.rs`) — whose GUI
  panel is itself deferred.

The embodiment this proposal is built around: **when the agent spawns a subagent, the GUI
opens a new tab bound to the child's session.** The tab shows the exact spawn prompt and
system context, streams every tool call live, displays the child's granted
extensions/skills/KBs, and accepts human input — the human can steer or interrogate the
subagent through the same chat box they use with any agent, and the parent can read the
child, steer the child, or abort it. Subagents become interoperable by both the AI and the
human, symmetrically.

---

## 2. Design principles

1. **The GUI is a view; the daemon is the truth.** Every workspace operation is defined at
   the session level first and works headless (CLI, server-only, GUI closed). The GUI
   attaches as a renderer of that truth. A tool call never *requires* a GUI to succeed —
   it degrades to the session-level effect and says so in its result.
2. **Reuse the proven agent→UI machinery.** Agent Drafter already ships an in-process MCP
   server whose tools push frames over a socket to a page, with a rebindable bridge
   registry and a blocking human round-trip
   (`crates/biorouter-mcp/src/agent_drafter/control.rs`,
   `crates/biorouter-server/src/routes/apps.rs`). This proposal instantiates the same
   pattern one level up — at the workspace, not inside one app.
3. **Do not rebuild what exists.** Conversation search is `chatrecall`; transcript reads
   are `GET /sessions/{id}`; tool-set mutation is `/agent/add_extension` +
   `/agent/remove_extension`; cancellation is `/agent/cancel` + `/agent/stop`; runaway-work
   discovery is `active_work`. The workspace extension is a thin, uniform tool surface over
   those, plus the two genuinely missing pieces (§4).
4. **Provenance everywhere.** Any message one agent injects into another conversation is
   permanently labeled with its origin. Cross-session control without provenance is
   indistinguishable from prompt injection.

---

## 3. What already exists (and what this proposal does with each)

This inventory is the reconciliation the feature request demands: before building, the
implementing agent must treat each row as binding — **reuse** rows are load-bearing
dependencies, not inspiration.

### 3.1 Backend control plane — reuse as-is

| Capability | Existing surface | Relationship |
|---|---|---|
| Start a conversation | `POST /agent/start` → `start_agent` (`routes/agent.rs:232`); `extension_overrides` selects the initial tool set | **Reuse** — `workspace_open` calls the same internal path |
| Inject a prompt (idle session) | `POST /reply` (`routes/reply.rs:415`), one turn per session enforced by `AppState.active_turns` (`state.rs:93,131`) | **Reuse** the turn lock; add a detached turn runner (§4.2) because `/reply` streams only to its own request |
| Inject mid-turn (steer a running session) | `POST /interrupt` → soft-interrupt queue (`routes/reply.rs:903`; 409 when idle) | **Reuse** — this is `workspace_send_prompt` with `mode:"steer"` |
| Cancel a turn / stop an agent | `POST /agent/cancel` (`reply.rs:959`), `POST /agent/stop` (`agent.rs:788`) | **Reuse** |
| Read any conversation, incl. tool calls | `GET /sessions/{id}` returns the full transcript; tool calls are `MessageContent::ToolRequest`/`ToolResponse` items inside `content_json` (`conversation/message.rs:189-203`; read path `session_manager.rs:2842-2896`) | **Reuse** — `workspace_read_conversation` is a projection over this, never a second storage path |
| Change a session's tool set at runtime | `POST /agent/add_extension` (`agent.rs:720`) / `remove_extension` (`agent.rs:756`); persisted per session in `extension_data` (`session/extension_data.rs:325-338`); inspect via `GET /agent/tools` (`agent.rs:549`) and `GET /sessions/{id}/extensions` (`session.rs:744`) | **Reuse** — the set is already mutable mid-session; `workspace_set_tools` wraps it |
| Scope knowledge bases per session | `KnowledgeService::set_active_for_session` (used from `apps.rs:1238-1247`) | **Reuse** |
| Discover/cancel running background work | `active_work` registry + `GET /active_work`, `POST /active_work/{id}/cancel` | **Reuse & feed** — workspace-spawned work registers here too; the deferred GUI panel becomes partially redundant with subagent tabs |
| Many concurrent agents in one daemon | `AgentManager` LRU keyed by session id (`execution/manager.rs:19-24`) | **Reuse** — the workspace registry *is* this registry |

### 3.2 Conversation recall — reuse, do not duplicate

**`chatrecall` already exists.** The disabled-by-default platform extension
(`crates/biorouter/src/agents/chatrecall_extension.rs`; registered under key `"chatrecall"`
in `PLATFORM_EXTENSIONS`, `agents/extension.rs:58-69`, `default_enabled: false`) exposes
one `chatrecall` tool with two modes: FTS5-ranked full-text **search** across all other
sessions (`chat_history_search.rs:84-150`, backed by the `messages_fts` mirror) and
**load** of a specific session's head/tail. The BR-17 design
([cross-session memory](cross-session-memory.md)) owns the recall roadmap.

Ruling for BR-71: the workspace extension **does not implement search**. Its server
instructions direct the model to `chatrecall` for *content* questions ("what did we
conclude about X last week?") and to `workspace_*` for *live control and structured reads*
(full transcripts, tool-call projections, running state). `workspace_read_conversation`
subsumes chatrecall's load mode with a richer projection, and its implementation calls the
same `SessionManager::get_session` — one storage read path. Enabling `workspace` should
suggest (not force) enabling `chatrecall`.

Also existing, and orthogonal — the instructions must name them so the model doesn't
misroute: `platform__ingest_conversation` (fold transcripts into a KB,
`platform_tools.rs:51-93`), `platform__read_session_blob` (within-session blob reads), and
the **Memory** MCP server (categorized long-term facts, not transcripts,
`biorouter-mcp/src/memory/mod.rs`).

### 3.3 Agent→UI control — the pattern donor

Agent Drafter's `appcontrol` server is the proven template, reused wholesale (pattern, not
code-sharing-by-force):

- In-process MCP server injected idempotently by name via
  `extension_manager.add_inprocess_server` (`apps.rs:1007`); tools emit
  `{"type":"ui","cmd":…}` frames through a single choke point (`UiBridge::emit`,
  `control.rs:763-780`).
- **Rebindable bridge registry** keyed by session (`UI_BRIDGES`, `apps.rs:483-496`), with
  generation-guarded `attach`/`detach` (`control.rs:709-754`), `cancel_all` unparking on
  disconnect, and state replay on reconnect.
- **Split socket** `select!`ing over agent events / UI commands / inbound frames
  (`apps.rs:2912`, `apps.rs:3698-3704`) — what makes a blocking `ui_ask` answerable
  mid-turn.
- **Blocking human round-trip**: mint request id → park on oneshot → resolve on inbound
  `ui_reply` (`control.rs:3599-3676`, `control.rs:887-899`).

BR-71 builds a sibling, `WorkspaceBridge`, with the same anatomy but a different scope:
one bridge **per GUI window** (not per app session), carrying workspace commands instead of
widget trees. Agent Drafter itself is untouched.

Also relevant precedents: the knowledge subagent loop streams every step of a
tool-embedded agent over SSE (`routes/knowledge.rs:1028-1079`, `SubAgentEvent`) — proof
that live sub-agent transparency fits the existing stack; Agent Drafter `consult` worker
profiles (`control.rs:4021`, `apps.rs:1748`) are named, capability-scoped worker agents
whose *browser-driven* turns are already streamed and stamped — but whose *agent-driven*
consults are opaque, the same gap in miniature; and the ACP crate
(`biorouter-acp`, Zed's Agent Client Protocol over stdio/WS) is a fully interactive
external agent surface, standalone from the GUI — untouched here, but a natural future
transport for workspace observation.

### 3.4 Frontend — the seams are already cut

- The tab/pane model is `ChatGroupsProvider` + reducer
  (`ui/desktop/src/contexts/ChatGroupsContext.tsx`,
  `components/chatGroups/chatGroupsReducer.ts`) with actions `openTab`, `activateTab`,
  `closeTab`, `moveTabToGroup`, split cap `MAX_GROUPS = 6`
  (`chatGroupsLayout.ts:18`). Crucially, `openTab` already **dedupes by session id** — an
  `openTab` for an already-open session activates that tab and focuses its group
  (`chatGroupsReducer.ts:277-288`). "Open or focus session X" is one existing dispatch.
- The menu-IPC → tab-state bridge pattern exists: `newTabRegistry.ts` /
  `closeActiveTabRegistry.ts` are singletons the provider registers claim functions on so
  IPC arriving outside `/pair` still works. The workspace command applier is a third
  registry of the same shape.
- **Cross-session reads need no tab.** `defaultChatStreamRegistry.getController(sessionId)`
  (`hooks/chatStreamStore.tsx:1226-1289`) yields any conversation's messages +
  pending tool calls renderer-side.
- New Electron windows: `create-chat-window` IPC → `createChat` (`main.ts:4339-4372`,
  window factory at `main.ts:992`), already able to resume a session id.
- The gap, confirmed: **no channel of the form "open/activate/close tab N by session id"
  exists from outside the renderer.** Deep links (`biorouter://`) open windows/views only;
  `?resumeSessionId=` URL sync (`useChatGroupsUrlSync.ts`) is renderer-internal.

### 3.5 The two genuinely missing pieces

1. **A session event broadcast.** Agent events currently flow only inside the
   `POST /reply` response that started the turn. Nothing can *observe* a session it didn't
   start — which is exactly what a subagent tab, a second window, or a parent-watching-child
   needs. (§4.2)
2. **A daemon→GUI command channel** carrying workspace commands, with the GUI echoing its
   layout back. (§4.3)

Everything else is composition.

---

## 4. The proposal

Three parts: the tool surface (4.1), the backend spine (4.2), the GUI channel (4.3), then
glass-box subagents as the integration of all three (4.5).

### 4.1 The `workspace` platform extension

A new **platform extension** (like `chatrecall`: in-process, no child process), registered
in `PLATFORM_EXTENSIONS` under key `"workspace"`, display name **"Workspace Control"**,
`default_enabled: false`. Seven tools:

#### `workspace_list`
List conversations and their live/GUI state.

```jsonc
{ "scope": "open" | "all" | "running",   // default "open": sessions with a GUI tab or a live agent
  "include_subagents": true }             // default true
```
Returns, per session: `session_id`, `name`, `session_type`, `working_dir`, `running`
(turn in flight, from `active_turns`), `parent_session_id` (subagents, §4.4), enabled
extension names, active KBs, and — when a GUI is attached — `gui: { window_id, tab_id,
focused }` from the layout echo (§4.3). Read-only; never blocks on the GUI (uses the last
echo).

#### `workspace_open`
Open a conversation in the workspace — new or existing.

```jsonc
{ "session_id": "…",                      // open/focus an existing conversation, OR
  "new": {                                 // start a fresh one
    "working_dir": "…",
    "extensions": ["developer", "knowledge"],   // extension_overrides, same semantics as /agent/start
    "knowledge_bases": ["kb-id"],
    "prompt": "…"                          // optional first user message
  },
  "placement": "tab" | "split" | "window", // default "tab"
  "focus": false }                          // default false: open in background, don't steal the user's focus
```
Session-level effect: for `new`, exactly `start_agent`'s path (create session, apply
overrides, optionally run the first turn detached §4.2). GUI effect: a `workspace` frame
(§4.3); `placement:"tab"` relies on the reducer's existing dedupe/adopt rules,
`"split"` maps to `moveTabToGroup` with an edge zone (refused with a clear message at
`MAX_GROUPS`), `"window"` maps to the `create-chat-window` IPC. Headless: returns
`{ session_id, gui_attached: false }` and says the session started without a GUI.

#### `workspace_read_conversation`
Structured read of any conversation.

```jsonc
{ "session_id": "…",
  "view": "transcript" | "tool_calls" | "summary" | "spawn_context",
  "range": { "last": 20 } | { "from_msg_uid": "…" },
  "max_chars": 20000 }
```
- `transcript`: messages with role/text/timestamps (tool payloads elided to one-line
  stubs).
- `tool_calls`: **only** the `ToolRequest`/`ToolResponse` projection — tool name,
  arguments, status, result digest, correlated by their shared id. This answers "what did
  that conversation's agent actually *do*" without transcript noise.
- `summary`: head/tail digest (chatrecall-load parity).
- `spawn_context`: for subagent sessions — the exact rendered system prompt, task
  instructions, and granted extensions/KBs it was started with (§4.4).

Implementation: one call to `get_session(id, true)` + projection. Oversized results are
retained in full by the existing large-result machinery rather than truncated silently.
Which mechanism carries them depends on size, and it is **not** the session blob in the
band a raised `max_chars` actually reaches:

- Above BR-6's `DEFAULT_LARGE_RESPONSE_TOKENS` (~10k tokens; the 200k-char `max_chars`
  ceiling is roughly 50k tokens of prose) the result is an ordinary extension-tool result,
  so `Agent::dispatch_tool_call` hands it to `large_response_handler::process_tool_response`,
  which writes the whole body to a handle under `<working_dir>/.biorouter/tool-output/` and
  returns a head/tail preview naming that path. The full payload never reaches persistence,
  so BR-7 never sees it.
- Below that budget the result is persisted intact, and BR-7 applies: a tool-response text
  item over `DEFAULT_BLOB_THRESHOLD_BYTES` (64 KB) moves to the `message_blobs` side table,
  hydrated back byte-for-byte on read (or left as a stub readable with
  `platform__read_session_blob` under `BIOROUTER_SESSION_BLOB_LAZY_LOAD`).

This ordering is BR-7's own stated design — its threshold sits "comfortably above anything
the BR-6 handler lets through". An earlier revision of this paragraph named only the session
blob, and the Task 13 handler repeated that claim in a comment and in the model-facing clip
marker; both were corrected. The binding requirement is the one that held throughout: the
payload is never silently truncated, and the reply always says where the rest is. Pinned by
`read_conversation_oversized_result_is_retained_in_full_on_the_production_path`.

#### `workspace_send_prompt`
Inject a prompt into another conversation.

```jsonc
{ "session_id": "…",
  "text": "…",
  "mode": "turn" | "steer" | "note",
  "wait": "none" | "final_message",        // default "none"
  "timeout_s": 120 }
```
- `turn`: target idle → start a detached turn (§4.2). Target busy → tool error naming the
  conflict (the caller can choose `steer`). Turn lock: `try_begin_turn_idempotent`.
- `steer`: target busy → the existing soft-interrupt queue (`/interrupt` semantics).
  Target idle → error, mirroring the 409.
- `note`: append a message to the target's history **without** triggering a turn
  (`user_visible: true`; picked up as context on the target's next turn) — how a parent
  leaves instructions for a paused child, or vice versa.
- `wait: "final_message"` parks the tool call (bounded, `ui_ask`-style) until the target's
  turn finishes and returns its final assistant message — giving the parent a synchronous
  ask-another-agent primitive without a new protocol.

**Provenance is mandatory.** Every injected message carries
`metadata.provenance = { kind: "agent_injection", from_session_id, from_session_name }`
(extending `MessageMetadata`, `message.rs:538-543`), rendered in the target transcript as
a visible "injected by …" chip. Not optional, not suppressible.

#### `workspace_set_tools`
Change what another conversation is allowed to use.

```jsonc
{ "session_id": "…",
  "add_extensions": ["…"], "remove_extensions": ["…"],
  "set_knowledge_bases": ["…"] }
```
Wraps `agent.add_extension`/`remove_extension` + persistence (exactly the
`/agent/add_extension` handler path) and `set_active_for_session`. Every change emits a
GUI notification on the target tab. Guardrails in §5.

#### `workspace_close`
```jsonc
{ "session_id": "…",
  "scope": "tab" | "turn" | "agent" }
```
- `tab`: GUI-only — close the tab (session and any running turn survive; matches today's
  close-tab semantics, `chatGroupsReducer.ts:404-407`).
- `turn`: `cancel_turn` (idempotent hard cancel of the in-flight turn).
- `agent`: `stop_agent` (cancel + evict from the registry); the session record remains.

#### `workspace_spawn_subagent`
The glass-box replacement surface for delegation (§4.5). Same parameter shape as today's
`subagent` tool (`instructions`, `subworkflow`, `parameters`, `extensions`, `settings`,
`background`) plus `visible: true` (default) and `placement`. When the workspace extension
is enabled, the server instructions steer the model to prefer this over bare `subagent`;
bare `subagent` remains for headless/compat.

> **As built (decisions 20 and 22): there is no `workspace_spawn_subagent`.** Two spawn
> tools with the same parameters and different names is a routing problem handed to the
> model, so the extension advertises **one** — `subagent`, keeping its existing name —
> which gained `visible` and `placement` and became the glass-box path. That is also what
> makes decision 21's two-tier surface possible: a session that may delegate is injected
> with `available_tools: ["subagent"]` and gets delegation without the cross-session
> control surface. The name survives below (and in §6's draft instruction block and §7's
> slice list) as the record of what was proposed; `WorkspaceClient::get_tools()` is the
> current truth.

### 4.2 Backend spine: detached turns and the session event broadcast

Two additions to `biorouter-server`/`biorouter`, useful far beyond this feature:

**Detached turn runner.** Factor the turn-driving loop out of the `/reply` handler so a
turn can be run server-side with no attached HTTP response: acquire the same
`active_turns` lock, call `agent.reply(...)`, and publish every `AgentEvent` to the
session's broadcast channel (below), persisting messages as today. `workspace_send_prompt
mode:"turn"`, `workspace_open.new.prompt`, and subagent turns (§4.5) all run through it.
`/reply` itself becomes "detached turn + a subscription that streams back to the caller,"
eliminating the current asymmetry where the request that starts a turn is the only party
that can see it.

**Session event broadcast + observer endpoint.** A per-session
`tokio::sync::broadcast` publisher (registered alongside the agent in `AgentManager`),
into which every turn — `/reply`-driven, detached, or subagent — publishes its
`AgentEvent`s. New route:

```
GET /sessions/{session_id}/events        → SSE observer stream (read-only)
```

Frames reuse the existing `MessageEvent` wire enum (`reply.rs:140-185`) so the generated
TS client and the renderer's SSE consumer (`chatStreamStore.tsx`) parse it unchanged. An
observer joining mid-turn receives a `UpdateConversation` snapshot first, then live
events. This is what lets a subagent tab, a second window, or a parent watching a child
render a turn none of them started. (After adding the route: `just generate-openapi && cd
ui/desktop && npm run generate-api`.)

### 4.3 The GUI command channel: `WorkspaceBridge`

**Route.** `GET /ui/workspace` WebSocket. Each Electron **window** connects once at
startup (from the renderer root, alongside `ChatGroupsProvider`), authenticating with the
server secret and identifying itself with a stable `window_id`.

**Two gates stand between the renderer and that route, and both were got wrong once.**
Neither can fail loudly: a blocked or refused socket leaves `is_attached()` false, so
`workspace_list` reports `gui_attached: false` and every tool degrades to its headless
arm — correct-looking output describing a GUI that is running in front of the user.

*The renderer's CSP must name the `ws` scheme, in both policies.* CSP relaxes a source
expression's scheme onto a URL's only via CSP3 §6.6.2.6's short list — `http`→`https`,
`ws`→`wss`/`http`/`https`, `wss`→`https` — so `http://127.0.0.1:*` does **not** cover
`ws://127.0.0.1:<port>/ui/workspace`, and a blanket `https:` does **not** cover the
`wss://` socket of an external TLS backend. Both `index.html`'s `<meta>` policy and the
header `buildConnectSrc()` attaches in `main.ts` apply to the window and the stricter
wins, so a source added to one and not the other still blocks the socket — which is
precisely how each of these two shipped. Measured in Electron 39.8.10 / Chromium 142
from a `file://` document: the first policy fires `securitypolicyviolation` with
`effectiveDirective: "connect-src"`, the shipped policy admits both sockets.
`ui/desktop/src/workspaceChannelCsp.test.ts` models the scheme rule and pins both files.

*The origin gate admits `file://`, and that is what the packaged renderer sends.*
`check_workspace_ws_auth` accepts a loopback origin (the dev renderer, served by vite) or
the literal `file://`, and **refuses `"null"`** — the opaque origin of every sandboxed
frame, including the agent-authored figures this same app renders in its artifact side
panel, put into a `srcDoc` iframe sandboxed without `allow-same-origin`. That made the
packaged path the one materially different input from the dev build, and it was
unmeasured until Task 31's fixup measured it
directly: a `file://` document in Electron 39.8.10 puts `Origin: file://` on the
WebSocket handshake, not `null`, so the packaged renderer is admitted. Re-measure if the
renderer ever moves to a custom protocol — a scheme the gate does not name reads as
cross-origin and takes the GUI half offline silently.

**Residual, recorded rather than fixed: this handshake is the one place the server
secret rides in a URL.** Every other renderer→daemon call sends it as an `X-Secret-Key`
header (`auth.rs`), which no access log records; `/ui/workspace` sends `?secret=…`
because the browser `WebSocket` constructor cannot set request headers — the route's own
module comment opens with that constraint. Loopback, where the URL never leaves the
machine, this is uninteresting. It becomes interesting in exactly the deployment the
`wss:` fix above first made reachable: an external TLS daemon, where the URL is encrypted
on the wire but is plaintext at both ends — a reverse proxy's access log, the daemon's
request logging — and the value logged is the master key `check_token` guards. The
conventional fix is to carry the token in `Sec-WebSocket-Protocol`, which the browser API
*can* set; that is a change to the route's auth shape and its renderer, not a fixup, so it
is named here rather than done. Until then: prefer loopback, and do not put a logging
proxy in front of a `biorouterd` you have pointed the desktop app at.

**Bridge.** `WorkspaceBridge`, modeled line-for-line on `UiBridge`
(`control.rs:557-663`): a registry keyed by `window_id`, generation-guarded
`attach`/`detach`, a pending-request map for blocking round trips, `cancel_all` on
disconnect. Multi-window aggregation lives above the registry: commands target a window
(default: the focused one, else most-recent), reads merge all windows' echoes.

**Outbound frames** (daemon → renderer), applied by a `workspaceCommandRegistry` (same
shape as `newTabRegistry`) that maps them onto existing reducer dispatches:

```jsonc
{ "type": "workspace", "cmd": "open_tab",    "session_id": "…", "focus": false, "placement": "tab" | "split" }
{ "type": "workspace", "cmd": "activate_tab","session_id": "…" }
{ "type": "workspace", "cmd": "close_tab",   "session_id": "…" }
{ "type": "workspace", "cmd": "open_window", "session_id": "…" }   // renderer relays to create-chat-window IPC
{ "type": "workspace", "cmd": "notify",      "session_id": "…", "level": "info", "message": "…" }
{ "type": "workspace", "cmd": "annotate_tab","session_id": "…", "badge": "subagent", "parent_session_id": "…" }
```

**Inbound frames** (renderer → daemon):

```jsonc
{ "type": "workspace_echo", "window_id": "…", "focused_session": "…",
  "layout": [ { "group_id": "…", "tabs": [ { "tab_id": "…", "session_id": "…", "title": "…" } ], "active_tab": "…" } ] }
{ "type": "workspace_result", "request_id": "…", "ok": true, "detail": "…" }
```

The renderer sends `workspace_echo` on every layout change (debounced) and on connect;
`workspace_result` resolves parked round trips (e.g. `workspace_open` reporting whether
the split was possible). Tabs opened this way bind sessions through the **existing**
`openTab` dedupe/adopt path — no parallel tab lifecycle.

A tab opened for a session the renderer isn't driving subscribes its
`ChatStreamController` to `GET /sessions/{id}/events` instead of owning a `/reply` stream
— the one renderer-side change to `chatStreamStore.tsx` beyond the command applier. The
controller owns its own reconnects, because the observer stream never "completes" from
the client's point of view: on stream end or transport error it re-subscribes with
backoff (1 s, doubling, capped at 15 s) until the tab detaches or the user takes the
session over. It must relay every frame in order and unchanged — the producer-side
ordering invariant that no `MessagesPersisted` precedes the `Message` frame carrying an
id it publishes is a property of the stream order, and only survives if each relay
preserves it.

Three things follow from the observer holding the controller's socket, and each of them
was wrong before it was written down. **One, an observer's subscription is not a turn.**
`canSubmitMessage`, `retryTurn` and the post-load chat-state settle all used a live
`abortController` to mean "a turn is running", which an observer has; reading it that way
swallowed everything typed into a daemon-opened tab and left a quiet observed session
showing a loading tab forever. They ask `hasLiveTurn()`, which excludes the feed, so the
takeover named above is reachable — and a takeover attempted while the observed agent is
mid-turn is answered by `/reply`'s existing 409, which the user can see. **Two, a dropped
observer connection is not a dead turn.** A stream ending without a `Finish` is this
loop's reconnect trigger, so it does not raise the `stream_interrupted` card a driver
would; the residual is that a `Finish` falling inside the reconnect gap leaves the
activity indicator stale until the next frame, since the reconnect's snapshot repairs the
transcript but says nothing about turn state. **Three, only the loop's owner may tear it
down.** `observeSession` refuses to take the socket from a live user turn (it is called
on daemon input, including `annotate_tab` for a tab that already exists), `stopObserving`
is a no-op on a controller that is driving, and the flag is generation-guarded and
cleared in a `finally` — a flag that outlives its loop makes every later attach
short-circuit, which is a permanently dead tab, because controllers are retained for the
life of the renderer. Closing the tab detaches, on any close and not only a daemon frame.

**Known, permanent consequence — an observer tab omits `expectedMessageIds`.** The store
sends that optimistic-concurrency guard on an in-place `edit_message` only while
`viewNamesEveryStoredRow` is true, and that flag is set in exactly the two places that
read a conversation back from the server, then cleared by any streamed event. An
observer-fed tab is a pure event consumer and never performs that read, so the flag is
always false for it and the guard is **permanently** omitted, where an ordinary tab sends
it after each read. This is safe — the guard is omitted, never falsified, and the
server-side cut still runs under the turn lock, bounded to the rows the handler itself
read — but it is a real capability difference, stated here rather than left to be
discovered. It must not be "fixed" by setting the flag for observers: an observer tab
genuinely does not know it holds every stored row. Consuming issue #59's
`MessagesPersisted` frame is what would earn the claim, and is a deliberate follow-up.

### 4.4 Session model additions

- `sessions.parent_session_id TEXT NULL` — set for subagent sessions at spawn (sibling of
  the existing `diverged_from` lineage column). Backfill: none needed.
- `list_session_summaries` gains an `include_subagents` flag (default false — existing
  behavior preserved) so History can show subagent transcripts grouped under their parent.
- Spawn context persistence: the child's rendered system prompt + task instructions are
  stored at spawn as its first message with `metadata { user_visible: true, agent_visible:
  false }` — visible in the tab, absent from the child's model context (which already
  receives it as the system override). This is what `view:"spawn_context"` and the tab
  header read.

### 4.5 Glass-box subagents — the embodiment

With 4.1–4.4 in place, the demonstration scenario is pure composition. When the parent
calls `workspace_spawn_subagent` (or `subagent` while the workspace extension is enabled
and a GUI is attached):

1. **Spawn** exactly as today (`subagent_handler.rs`: fresh `Agent`, parent's provider,
   parent's extensions minus exclusions, `subagent_system.md` override, `SessionType::
   SubAgent`, active-work registration, recursion guard intact) — plus
   `parent_session_id` and the persisted spawn-context message.
2. **Run the child through the detached turn runner**, so its `AgentEvent`s publish to its
   session broadcast instead of being privately consumed. The parent's tool call still
   parks on completion and still receives only the `SubagentResult` summary — the parent's
   context stays clean; the *transparency* is on the observation plane, not in the
   parent's context window.
3. **Announce over the WorkspaceBridge**: `open_tab { session_id: child, focus: false }` +
   `annotate_tab { badge: "subagent", parent_session_id }`. The tab opens in the
   background (never stealing the user's composer), streams the child live via
   `/sessions/{child}/events`, and its header shows: spawned-by link, the spawn context
   (expandable), and the child's granted extensions/skills/KBs (from
   `GET /sessions/{id}/extensions` + KB state).
4. **The human can intervene** through the tab's ordinary chat box. While the child's turn
   runs, input goes down the existing soft-interrupt path (`steer`); between turns it can
   start a new child turn or leave a `note`. Human interventions carry provenance
   `{ kind: "user_direct" }`; the parent, on resolving its tool call, is told whether the
   human intervened so it can weigh the summary accordingly.
5. **Both sides can end it.** The parent aborts via `workspace_close { scope:"turn"|"agent" }`;
   the human via a Stop control on the tab.
   Closing the tab alone never kills the child (consistent with existing tab semantics);
   stopping the child resolves the parent's tool call with `SubagentStatus::Incomplete` +
   whatever partial summary exists.
6. **Reporting back** is unchanged (final-summary envelope), with `workspace_read_conversation
   view:"tool_calls"` available when the parent wants the child's actual actions rather
   than its self-report — and available to the human for the same audit.

`BIOROUTER_SUBAGENT_BACKGROUND` handles (`subagent_handle.rs`) compose unchanged: a
background child is simply a live tab whose parent isn't parked on it.

Out of scope for BR-71 but deliberately enabled by it: routing Agent Drafter `consult`
worker turns through the same observation plane, and exposing workspace observation over
ACP.

---

## 5. Permissions, safety, and abuse resistance

An agent that can read other conversations, inject prompts into them, and change their
tool sets is a materially new capability. Per `.github/copilot-instructions.md`, the
permission-relevant code below requires human review regardless of AI assistance.

- **Off by default.** `default_enabled: false`, like `chatrecall`. Enabling it is an
  explicit user decision surfaced with a capability summary.
- **Permission-mode integration.** In manual/smart-approval modes, every mutating
  `workspace_*` call (open/new, send_prompt, set_tools, close, spawn) is confirmable like
  any sensitive tool; `workspace_list`/`read_conversation` follow read-tool grading via
  the existing `get_tools` permission grading (`agent.rs:607-644`). In autonomous mode,
  mutations proceed but are **always** GUI-visible (toast on the target tab + provenance
  chips) — silent cross-session action is not a supported configuration.
- **Provenance is structural** (§4.1). Injected prompts and steers are labeled in
  storage, not just in the UI.
- **No covert reads.** `read_conversation` refuses `Hidden` sessions and honors the same
  visibility rules as the session list; subagent transcripts are readable by their parent
  and by the user, and (default) by other sessions only when `include_subagents` scope is
  requested. Transcripts can contain secrets pasted by the user — the instructions warn
  the model to treat cross-session reads as sensitive, and reads are logged as
  tool calls in the *reading* session (auditable via the same tool-call projection).
- **No self-escalation.** `workspace_set_tools` targeting the caller's own session follows
  the caller's permission mode exactly as a user-initiated extension toggle would; in
  approval modes it always prompts. Removing security-relevant extensions or adding
  process-spawning ones on *any* target surfaces a confirmation regardless of mode.
- **Subagents never get the workspace extension** (extension of the existing "subagents
  cannot create subagents" guard, `agent.rs:2046-2055`) — no delegation-tree fan-out of
  workspace control, no child steering its parent.
- **Bounded fan-out.** Spawn caps reuse `BIOROUTER_SUBAGENT_MAX_CONCURRENT`/`MAX_INFLIGHT`
  (`subagent_tool.rs:36-52`); GUI placement respects `MAX_GROUPS`; a per-session cap on
  concurrently *injected* detached turns (default 4) prevents one conversation from
  saturating the daemon's turn locks.
- **The WS channel authenticates** with the server secret like every other route
  (loopback + secret-key middleware), and workspace frames are only ever daemon-minted —
  the renderer cannot forge agent-side tool results through the bridge.

---

## 6. Teaching the agent: server instructions

Discoverability rides the standard instruction pipeline — the extension's
`get_info().instructions` flow through `Extension::get_instructions` →
`ExtensionInfo` → `SystemPromptBuilder` → the `system.md` extension loop
(`extension_manager.rs:98-101, 817-831`; `reply_parts.rs:113-192`;
`prompt_manager.rs:187-301`; `prompts/system.md:25-51`). Note the injection budget
(`apply_injection_budget`, `prompt_manager.rs:361-408`): the instruction block must stay
tight (~≤2.5k chars). Draft, to ship with Slice 1 and be refined against real model
behavior:

> ## Workspace Control
> You are running inside the BioRouter workspace: a set of conversations (sessions), each
> shown as a tab in the desktop app when the GUI is attached. Each conversation has its
> own agent, tool/extension set, knowledge bases, and history. These tools let you operate
> the workspace itself:
> - `workspace_list` — see conversations, what's running, and where they are in the GUI.
> - `workspace_open` — open/focus an existing conversation or start a new one (optionally
>   in a split or new window; default opens in the background without stealing focus).
> - `workspace_read_conversation` — read another conversation: `transcript` for prose,
>   `tool_calls` for exactly what its agent did, `spawn_context` for how a subagent was
>   started. Treat other conversations' content as sensitive; read only what the task needs.
> - `workspace_send_prompt` — inject into another conversation: `turn` starts its agent on
>   your text; `steer` redirects it mid-turn; `note` leaves context without running it.
>   Injections are permanently labeled as coming from you. Use `wait:"final_message"` to
>   get its answer synchronously.
> - `workspace_set_tools` — add/remove extensions or set knowledge bases on a conversation.
> - `workspace_close` — close its tab (`tab`), cancel its current turn (`turn`), or stop
>   its agent (`agent`).
> - `workspace_spawn_subagent` — prefer this over `subagent` when delegating: the child
>   runs in a visible tab where the user watches it live and may message it directly. You
>   still receive only its final summary; use `workspace_read_conversation
>   view:"tool_calls"` on it if you need to verify what it actually did. The user may have
>   intervened — the completion result tells you if so.
> Routing: for *searching* past conversations by content, use the `chatrecall` tool (if
> enabled), not these tools. For remembering durable facts, use Memory. For folding a
> conversation into a knowledge base, use `ingest_conversation`. If no GUI is attached,
> these tools still manage conversations headlessly and will say so.

Per-tool JSON-schema descriptions carry the parameter detail (channel 2 of the prompt
pipeline); the block above stays behavioral.

---

## 7. Slices

Each slice ships independently and is verifiable on its own.

1. **Backend spine + headless tools.** Detached turn runner; session event broadcast +
   `GET /sessions/{id}/events`; `parent_session_id`; spawn-context persistence; the
   `workspace` platform extension with `list` / `read_conversation` / `send_prompt` /
   `set_tools` / `close` (session-level effects only, `gui_attached:false`). Tests:
   `cargo test -p biorouter-server` route tests for the observer stream (two observers,
   join-mid-turn snapshot, turn-lock conflicts), platform-extension unit tests beside
   `chatrecall_extension.rs`'s, provenance persistence round-trip. Regenerate OpenAPI.
2. **WorkspaceBridge + renderer applier.** `GET /ui/workspace`, the per-window bridge
   registry (generation-guard tests modeled on the `apps.rs` reconnect tests),
   `workspaceCommandRegistry` + reducer wiring, layout echo, observer-backed
   `ChatStreamController` mode, provenance chips and set-tools toasts. `workspace_open`/
   `close(tab)` gain their GUI effects. Vitest reducer/applier tests; verify live per the
   dev-GUI rules (`BIOROUTER_NO_HMR=1`, CDP screenshots — see
   [launching the dev GUI](../../desktop-ui/launching-the-dev-gui.md)).
3. **Glass-box subagents.** Route subagent execution through the detached runner;
   `workspace_spawn_subagent`; auto-open + badge; tab header (spawn context, extensions,
   KBs); human steer path + intervention flag in the parent's result; Stop control;
   `include_subagents` in History grouped by parent. E2E: a scripted parent spawns a
   child, the harness asserts the child tab streams tool frames and a human `steer`
   reaches it (pattern: `scripts/agent-drafter/ui-control-harness.mjs`).
4. **Polish + docs.** `workspace_list`/active-work cross-links, instruction-text tuning
   against real model behavior, user docs under `docs/agent-loop/subagents.md` +
   `docs/extensions/built-in/workspace.md`, and an update to
   [tool-routing](../tool-routing.md) for the chatrecall/workspace split.

---

## 8. Open questions — all resolved

Each question is kept in place with its answer and the decision that settled it, rather
than deleted: the question is what makes the answer legible.

1. **Focus etiquette.** *Should the user be able to set "never open tabs automatically"?*
   **Yes — decision 7, built in Task 29.** One switch, `WORKSPACE_ANNOUNCE_ONLY`
   (Settings → App → Workspace), **off** by default, so background-open remains the
   design's default. When on, the daemon downgrades every focus-stealing frame —
   `open_tab`, `open_window` *and* `activate_tab`, since yanking the view to an existing
   tab is the same intrusion by another route — to a `notify`, and the tool result tells
   the model no tab was opened so it cannot claim otherwise. Subagent spawns resolve the
   setting before claiming a visible-tab slot, so announce-only children never consume
   the decision-26 cap.
2. **`workspace_send_prompt wait:"final_message"` vs. Agent Drafter `consult`.**
   **Unified — decision 13, done in Task 41.** `consult` was not re-expressed as a
   workspace *tool call*; it kept its own contract (name, params, depth-1, per-profile
   timeout, blocking answer, error envelopes) and was moved onto this spine underneath:
   a consulted worker's turn takes the server turn lock, registers in `AgentManager`, and
   publishes to the session event bus, so it is observable via
   `GET /sessions/{id}/events`, steerable via `POST /interrupt`, and cancellable via
   `workspace_close scope:"turn"` — exactly like a glass-box subagent. The resolution
   paragraph lives with the apps platform owners, in
   [apps platform design](../../agent-drafter/apps-platform-design.md) (§ "`consult` runs
   on the BR-71 workspace spine").
3. **Cross-window targeting.** *Is the focused-else-most-recent default good enough, or
   is a `window_id` parameter needed?* **The heuristic stands — decision 15.** Task 22
   implements `focused_or_recent` as proposed; no `window_id` parameter was added, and
   `workspace_open` gained `placement: tab|split|window` instead, which is what the
   observed requests actually wanted.
4. **Observer backpressure.** *The `Lagged` resync is spec'd, but what does it cost?*
   **Implemented in Task 7 and measured — decision 16.** Both consumers (the observer
   stream and `/reply`) answer `RecvError::Lagged` with a `bus_lag_resync_frame`
   `UpdateConversation` read from storage rather than silently skipping frames.
   Measured by `scripts/workspace/glassbox-harness.mjs`: a fresh observer's first
   snapshot arrived in **3 ms for a 1-message session** (debug `biorouterd`, macOS arm64,
   2026-07-31). Read that with its subject attached — it is the floor, not evidence that
   resync is free at any transcript length, which is why the harness prints the message
   count beside the latency and this bullet repeats it.

   ⚠ **And read it as a proxy, not as the `Lagged` arm.** The harness opens a fresh
   observer and times its first frame, which is the unconditional join-mid-turn snapshot
   at the top of the stream task — not the `RecvError::Lagged` branch below it. The two
   are timed because they do the same dominant work: `bus_lag_resync_frame` is a
   `session_manager().get_session(id, true)` plus one `UpdateConversation`, which is the
   same storage read the initial snapshot performs. What the harness therefore does
   **not** cover is that the lag branch is reached at all — delete it and this
   measurement stays green.

   Coverage of the branch itself lives in unit tests, and only for one of the two
   consumers. `reply::a_lagged_sse_loop_resyncs_the_client_from_storage` overruns the
   real broadcast ring (`BUS_CAPACITY + 1` publishes with nothing reading) so the loop's
   first `recv` deterministically returns `Lagged`, and asserts the client is sent the
   stored conversation; replacing the resync with a bare `continue` makes it time out.
   The **observer stream**'s identical arm in `session_events.rs` has no such test — that
   is the real gap here, and it is a gap in tests, not in the implementation.
5. **CLI surface.** *Worth building `session watch` / `session send` as free verification
   tooling?* **Yes — decision 9, built in Task 20**, and it grew past the question:
   `session watch`, `session send`, `session list --subagents`, `session attach`
   (`--of`, `--read-only`) and `session cancel`. Task 42b then made it structural — a
   compile-time parity gate in `biorouter-cli` fails the build when a workspace
   capability has no CLI counterpart or a written, bounded exception.

## Related documentation

- [Subagents (user guide)](../subagents.md) — today's delegation surface this proposal makes transparent.
- [Cross-session memory (BR-17)](cross-session-memory.md) — owns recall/search; BR-71 deliberately does not.
- [Session branching (BR-45)](session-branching.md) — the `msg_uid` identities `workspace_read_conversation` ranges rely on.
- [Apps platform design](../../agent-drafter/apps-platform-design.md) — the `appcontrol`/`UiBridge` pattern this generalizes.
- [Tool routing](../tool-routing.md) — must gain the workspace/chatrecall routing row when Slice 1 ships.
