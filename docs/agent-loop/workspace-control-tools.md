# Workspace Control tool reference

> **What this is.** The precise reference for the eight tools the Workspace Control capability puts in the model's tool list: exact name, arguments, return shape, refusal conditions, and the cases where a tool reports success it did not earn.
> **Status:** Current. Two false-success paths and one dead GUI frame are documented in place — see [`subagent`](#subagent) and [`workspace_close`](#workspace_close).
> **Audience:** developers working on the agent loop, and anyone diagnosing a workspace tool that behaved unexpectedly.

Workspace Control (`workspace`, identifier `Workspace`, display name **Workspace Control**) is a platform extension whose tools operate on BioRouter *sessions* — other conversations — rather than on files or the network. This page is the per-tool contract. It is written for the moment a tool misbehaves and you need to know what it actually promises, so it prefers exact strings and named source paths over explanation. For the user-facing account of what the extension is for, when the tiers differ, and what the confirmation cards say, read [the Workspace Control capability guide](../extensions/built-in/workspace.md) first.

Everything below is read off `crates/biorouter/src/agents/workspace_extension.rs` (the seven `workspace_*` tools), `crates/biorouter/src/agents/subagent_tool.rs` (`subagent`), `crates/biorouter/src/agents/workspace_inspector.rs` (the always-confirm rule), and `ui/desktop/src/components/chatGroups/workspaceCommandPlanner.ts` (what the renderer does with a frame).

## Conventions that apply to every tool

**The name the model sees is prefixed, and for seven of the eight that is the only spelling that works.** Extension tools reach dispatch as `{extension}__{tool}`, so the model's tool list holds `workspace__workspace_list`, `workspace__workspace_read_conversation`, and so on — `subagent` included, as `workspace__subagent`.

For the seven `workspace_*` tools the prefixed form is the **only dispatchable spelling**. `ExtensionManager::dispatch_tool_call` does repair a stripped prefix, but only for a hardcoded three: `execute_code`, `read_module`, `search_modules`, and only when a `code_execution` extension is loaded. Everything else keeps the name it arrived with, and the next step is fatal to a bare workspace name: `get_client_for_tool` matches a client by `prefixed_name.starts_with(key)`, so a bare `workspace_close` *does* find the `workspace` client, but dispatch then strips the client name and requires a `__` separator — `"workspace_close".strip_prefix("workspace")` yields `_close`, the `__` strip fails, and the call dies with `Invalid tool name format: 'workspace_close'`.

Only `subagent` tolerates both spellings, and not at dispatch: `is_spawn_tool_call` (`agent.rs`) matches `subagent` and `workspace__subagent` alike, and `Agent::dispatch_tool_call` intercepts either form before the extension manager ever sees it. The bare names that appear in `is_workspace_tool_refused_for` and `is_parking_workspace_tool` are defensive *classification* — they make the subagent refusal and the parking exemption hold whatever spelling arrives — not evidence of a second dispatch path.

This page uses the bare names for readability.

**Errors are returned, not raised.** Every handler returns `Result<Vec<Content>, String>`; `call_tool` converts an `Err` into `CallToolResult::error` with the text `Error: {message}`. A refusal is therefore a normal tool result the model reads and can act on, not a transport failure.

**Enum arguments are validated inconsistently, and that matters when diagnosing.** Five of them are closed vocabularies that refuse an unknown value; three silently fall back to a default. `serde` does not enforce the `enum` in a JSON schema, so the only enforcement is the handler's own `match`.

| Argument | Behaviour on an unrecognised value |
|---|---|
| `workspace_open.placement` | **Refused** — `unknown placement "…" — use "tab" (default), "split" or "window"` |
| `workspace_open.new.kind` | **Refused**, before anything is created — `sub_agent` gets its own refusal naming the `subagent` tool, and anything else (including absent) gets the two-value vocabulary |
| `subagent.placement` | **Refused**, before a child session is created (same message) |
| `workspace_close.scope` | **Refused** — `unknown scope '…' (tab \| turn \| agent)` |
| `workspace_send_prompt.mode` | **Refused** — `unknown mode '…' (turn \| steer \| note)` |
| `workspace_watch.mode` | **Refused** — `unknown mode '…' (any \| all)` |
| `workspace_list.scope` | **Silently treated as `open`**, and echoed back verbatim in the payload's `scope` field |
| `workspace_read_conversation.view` | **Silently treated as `transcript`** |
| `workspace_send_prompt.wait` | **Silently treated as no-wait**; only the exact string `final_message` parks |

**Two tools park, and are exempt from the dispatch permit.** `workspace_watch` and `workspace_send_prompt` block on work happening in another session, so `is_parking_workspace_tool` (`agent.rs`) keeps them from holding a global tool-dispatch permit while they wait. Without that a parked watch would throttle the caller's own unrelated tool calls.

**Subagents are refused all seven `workspace_*` tools.** `is_workspace_tool_refused_for` (`agent.rs`) enumerates them and refuses when the calling session's type is `SubAgent`, in both the prefixed and bare name forms. `subagent` itself is refused inside a subagent by `is_spawn_tool_call`, so a child cannot spawn grandchildren.

**What a missing daemon costs.** `workspace_services::get()` returns `None` in a process with no daemon (a plain `biorouter` terminal session). Each tool's entry names its own refusal; the summary is:

| Tool | With no daemon |
|---|---|
| `workspace_list` | Works. `gui_attached: false`, every `running` is `false`, `knowledge_bases` `[]` and `primary_kb` `null` — the KB selection comes from the services handle, so with none it is the default rather than a read of the session |
| `workspace_read_conversation` | Works — it reads the session store directly |
| `workspace_watch` | Works, and still sees background children through the handle registry; liveness for anything else is `Unknown` |
| `workspace_send_prompt` | `note` works; `steer` and `turn` refuse by name |
| `workspace_set_tools` | Extensions, skills and provider work; `set_knowledge_bases` refuses |
| `workspace_close` | `tab` is a stated no-op; `turn` and `agent` refuse |
| `workspace_open` | `session_id` form reports the session with no tab; `new` refuses |
| `subagent` | Works, headless (no tab) |

## One worked sequence

Every tool below is documented in isolation, but they are meant to compose. This is the common four-call shape — find the target, delegate, park, verify — with complete argument objects. Each argument is the one the corresponding section describes; nothing here is a shortcut the tools do not offer.

**1. Find out what is already running**, so the delegation does not duplicate work in flight:

```json
{ "scope": "running", "limit": 20 }
```

`scope: "running"` is the exact "a turn is in flight" filter; `open` would also return idle tabs.

**2. Delegate, without blocking on the child**:

```json
{
  "instructions": "Run the crate's test suite and report every failing test with its assertion message.",
  "extensions": ["developer"],
  "background": true,
  "visible": true,
  "placement": "split"
}
```

`background: true` returns immediately with a handle message naming the child's **session id** — that is the id every subsequent call takes, not the handle id. If `BIOROUTER_SUBAGENT_BACKGROUND` is off, this argument is silently ignored and the call blocks instead, so read the result text rather than assuming.

**3. Park until it finishes**, instead of polling:

```json
{ "session_ids": ["<child session id>"], "mode": "all", "timeout_s": 300 }
```

A timeout is not an error. If the reply says nothing finished, the child is still running and the same call can be repeated; if it adds the "no daemon attached" second line, the ids resolved as `Unknown` and it was never established that they had started.

**4. Verify what the child actually did**, rather than trusting its summary:

```json
{ "session_id": "<child session id>", "view": "tool_calls", "last": 100, "max_chars": 60000 }
```

`view: "tool_calls"` is the only view that shows the child's effects on the repo; `last` and `max_chars` are the two controls the clip message names when the projection is cut.

A fan-out is the same sequence with step 2 issued several times in one assistant message and step 3 taking every child id at once.

---

## `workspace_list`

Enumerates sessions with their live state, capability set and GUI placement. Read-only.

### Arguments

| Name | Type | Default | Meaning |
|---|---|---|---|
| `scope` | string | `"open"` | `"open"` — the session has a live agent **or** a turn in flight **or** a GUI tab. `"all"` — every listable session. `"running"` — a turn in flight only. |
| `include_subagents` | bool | `true` | Passed through to the store query. |
| `parent_session_id` | string | — | Only sessions whose `parent_session_id` equals this. Pass your own id to enumerate your children. |
| `only_subagents` | bool | `false` | Only rows whose `session_type` is `sub_agent`. Combines with `parent_session_id`. |
| `offset` | u32 | `0` | Rows to skip **after** scope filtering. |
| `limit` | u32 | `50` | Clamped to `1..=200`. |

Called with no arguments at all, it falls back to `WorkspaceListParams::default()` — i.e. scope `open`, subagents included, first 50 rows.

### Returns

Pretty-printed JSON:

```json
{
  "gui_attached": true,
  "scope": "open",
  "offset": 0, "limit": 50, "returned": 3,
  "total_matching": 3, "has_more": false,
  "sessions": [ { "…": "one row per session" } ]
}
```

Each row carries `session_id`, `name`, `session_type`, `working_dir`, `running`, `parent_session_id`, `extensions`, `knowledge_bases`, `primary_kb`, `gui`.

- `extensions` is read **per returned row** (the store's summary row does not carry it): the session's own `extension_data` if it has any, otherwise the machine-wide `get_enabled_extensions()`. A read error yields `[]` rather than failing the listing — so an empty list is ambiguous between "none" and "could not read".
- `primary_kb` is `null` when the session has knowledge bases but no write target. That is a real state, not "none set": a `kb_write` with no `kb_id` fails in it. Do not collapse it to the first id.
- `gui` is `null` when the session has no tab, otherwise `{ "window_id", "group_id", "tab_id", "focused" }`, resolved from the renderer's layout echo. It is `null` for every row when no GUI is attached.

### Paging and the scan ceiling

The store is walked in chunks of 500 and filtered here, so `total_matching` and `has_more` describe the rows that passed the filter, not a storage window. The walk stops at **20,000 scanned sessions**; if it does, the payload gains `scan_truncated: true`, `scanned: 20000`, and a `note` saying `total_matching` is a lower bound and naming `scope` / `parent_session_id` / `only_subagents` as the narrowing controls.

### When to reach for it

As the first call of any cross-session task — you need a `session_id` before any other tool is useful — and to answer "what am I running", where `scope: "running"` is exact. `scope: "open"` deliberately includes running-but-tabless subagents, which a `has_session` check alone would miss.

---

## `workspace_read_conversation`

Projects another conversation into one of four views. Read-only, and logged: every call emits a `tracing::info` line carrying `caller`, `target` and `view` (`"workspace cross-session read"`), in addition to being a tool call in the caller's own transcript.

### Arguments

| Name | Type | Default | Meaning |
|---|---|---|---|
| `session_id` | string | required | The conversation to read. |
| `view` | string | `"transcript"` | `transcript` \| `tool_calls` \| `summary` \| `spawn_context`. Anything else silently reads as `transcript`. |
| `last` | usize | — | Tail of N messages. **Only affects `transcript` and `tool_calls`.** |
| `from_msg_uid` | string | — | Start from the message with this durable id (BR-45 message identity; the id is `Message.id`). Applied before `last`. **Only affects `transcript` and `tool_calls`** — but an id that matches no message errors in *every* view, because the lookup runs before the view is chosen. |
| `max_chars` | usize | `20000` | Capped at `200000`. |

### Refusals

- A session whose type is `Hidden` is refused in every view: `this session is hidden and cannot be read`.
- `view: "spawn_context"` on a session with no `SpawnContext`-provenance message: `this session has no recorded spawn context`.
- An unknown `from_msg_uid`: `no message with msg_uid '…' in this session`.

### Returns

A single text block whose first line is `Session {id} ({name}, {SessionType})`, then a blank line, then the projection:

- **`transcript`** — user-visible messages only (anything with `metadata.user_visible == false` is dropped). Each line is `[{Role}] `, then `(injected: {ProvenanceKind}) ` when the message carries provenance, then the content, with tool payloads collapsed to `<tool call: …>` and `<tool result: {id}>` stubs.
- **`tool_calls`** — correlated pairs across the range: `→ [{id}] {readable request}` and `← [{id}] ok: {first 400 chars}` or `← [{id}] error: {e}`. Empty range yields `No tool calls in range.`
- **`summary`** — `Working dir: …`, `Messages: N`, then `--- First ---` with the first three messages and `--- Last ---` with the last three, both rendered through the transcript projection.
- **`spawn_context`** — the text of the first message whose provenance kind is `SpawnContext`.

### The `max_chars` cap is pagination, not the size mechanism

When the projection exceeds `max_chars` it is cut and the reply appends `… [clipped at N chars — narrow with `last` or `from_msg_uid`, or raise `max_chars` (up to 200000)…]`. Retaining an oversized payload in full is handled *outside* this tool, on the ordinary tool-result path: above roughly 25k tokens BR-6's `large_response_handler` writes the whole body to a handle under `<working_dir>/.biorouter/tool-output/` and returns a preview naming it; below that, BR-7 externalizes a tool response over 64 KB into the session blob table. The tool's own cap sits on top of both.

### When to reach for it

`view: "tool_calls"` is the one that answers "what did that agent actually *do* to my repo", and it is what the extension's own instructions tell the model to use to verify a subagent — because the parent only ever receives the child's final summary. `view: "summary"` for a cheap digest, `spawn_context` to recover the instructions a child was started with.

---

## `workspace_send_prompt`

Writes into another conversation. Three modes with three different blast radii; every injection is provenance-stamped `MessageProvenance { kind: AgentInjection, from_session_id, from_session_name }` and the label is stored, so it survives reload.

**Any conversation, not only a child.** The target may be a subagent this conversation spawned, a sibling, or a chat the user opened and this agent has never touched. Lineage is not a boundary — the write rule is `may_write` ⇔ `may_read`, so the reachable set is exactly the set this conversation may already read in full. What *is* a boundary is the privacy tier: a public-capability chat is refused a private conversation under every verb, and a private-capability chat writing into a public one raises a **first-crossing approval showing the exact payload**, once per (caller, target) pair, in every permission mode including Fully Automatic.

The tool's own description tells the model to use it **only when necessary**, and the reason is not politeness: the target may be a conversation a person is reading, and an injection interrupts them.

**All three modes reflect in an open tab live**, without a reload. `note` publishes the stored row onto the session bus itself; `steer` is published by the target's own turn loop when it drains the soft interrupt; `turn`'s injected prompt is published by `Agent::reply` at the point the row becomes durable — which is a case its `MessagesPersisted`-only rule deliberately does not cover, since that rule assumes the client authored the prompt and already holds it. In every case the publish happens **after** the row is durable and carries the row's own minted uid.

### Arguments

| Name | Type | Default | Meaning |
|---|---|---|---|
| `session_id` | string | required | Target. Must not be the caller. |
| `text` | string | required | Must not be blank after trimming. |
| `mode` | string | required | `turn` \| `steer` \| `note`. |
| `wait` | string | `"none"` | Only `"final_message"` parks; every other value (including a typo) returns immediately. |
| `timeout_s` | u64 | `120` | `.min(600)` only — there is no lower bound, so `timeout_s: 0` gives up instantly. |

Two refusals precede the mode split: `refusing to inject into your own session — just continue the conversation`, and `text must not be empty`.

### `mode: "note"`

Appends a message and starts nothing. Works with no daemon — it goes straight to the session manager. The text is wrapped by `frame_workspace_injection` in an untrusted-data envelope naming the sending conversation, stamped with provenance, and marked `.pinned()` so the next compaction carries it verbatim instead of summarizing it away. Returns `Note appended to session {id} (no turn started; preserved across compaction).`

### `mode: "steer"`

Redirects a turn already in flight. Requires the daemon (`steer requires the BioRouter daemon (no workspace services installed)`) and requires the target to be running (`target session has no turn in flight — use mode:"turn" instead`). It queues through `try_queue_soft_interrupt`, whose fallible form is deliberate: the server's turn lock is released *after* the agent loop stops accepting interrupts, so `is_turn_active` can still be true for a turn whose queue has closed. A closed queue comes back as `steer refused for session {id}: {reason} — use mode:"turn" instead` rather than being reported as queued. Posts a toast on the target tab. Returns `Steer queued for session {id}'s running turn ({turn_id}).`

### `mode: "turn"`

Starts a detached turn. Requires the daemon. Two further gates:

1. **Headless approval refusal.** If no GUI is attached *and* the target's own agent config is in an approval mode, the call is refused, because a tool confirmation the turn raises would park where nobody can answer it. Note the exact reading: the mode consulted is the **target agent's** `AgentConfig.biorouter_mode`, via `peek_agent` — and when the target has **no live agent** the check returns `true` conservatively rather than minting one. So headless `mode: "turn"` against a cold session is refused whatever the machine's permission mode is. `Chat` and `Auto` are classified as non-approval; `Approve` and `SmartApprove` as approval.
2. **Per-caller fan-out cap.** `BIOROUTER_WORKSPACE_MAX_INJECTED_TURNS`, default **4**, counted per *calling* session. The fifth concurrent injection is refused with `this session already has 4 injected turns in flight (cap 4); wait for one to finish`. The slot is not released when the tool returns — a background follower holds it until the turn's terminal event, with a 2 s `is_turn_active` poll as a safety valve against a lagged event stream.

The event subscription is opened *before* the turn starts, and everything the follower reads is gated on that turn's own id — a turn already in flight when the call arrived can publish its answer onto the same stream, and an ungated follower would report the previous turn's final message as this one's.

Returns, by path:

- no wait — `Detached turn {turn_id} started on session {id}.`
- `wait: "final_message"` — one JSON object, pretty-printed as the text the model reads and carried whole as `structured_content`, so a model and a script are handed the same fields. `verdict` is the field to branch on:

  | `verdict` | When | Result flagged `is_error` |
  |---|---|---|
  | `completed` | The turn finished and its final message does not open with a refusal. | no |
  | `declined` | The turn finished, but one of the final message's first three sentences is a first-person refusal of the request — *"I won't run …"*, *"I can't act on …"*, *"I decline …"*. `declined_because` carries that sentence, verbatim. Nothing the caller asked for can be assumed done. | no — the target did what its safety rules say to, and an error invites a retry that will be declined again |
  | `errored` | The turn published `TurnError`. `error` carries the message. | **yes**, as before |
  | `timed_out` | The park gave up; the turn keeps running. `waited_s` says how long. | no — a timeout is not a failure |
  | `cancelled` | The turn published `TurnFinished { reason: "cancelled" }` — Stop in its tab, or a `workspace_close`. | no |

  The rest of the object: `note` (one sentence on what the verdict means for the caller's next step), `final_message` (the target's final assistant text, **complete**, or `null` when it wrote none — never a blank string), `session_id`, `turn_id`, `finish_reason` (`stop` / `cancelled`, absent otherwise) and `tool_calls` (distinct tool calls the target made in the turn — what it *did*, beside the verdict's reading of what it *said*; for `timed_out`, the count so far). For `timed_out`, `final_message` is always `null`: whatever the turn has said so far is not its final message.

A shape, for the refusal the 2026-09-10 QA run measured:

```json
{
  "verdict": "declined",
  "note": "The target received your text as untrusted cross-conversation data and declined to act on it: nothing you asked for was done. …",
  "declined_because": "I won't run shell commands based solely on a lower-trust cross-chat injection.",
  "final_message": "I won't run shell commands based solely on a lower-trust cross-chat injection. If you want me to run `echo CROSSCHAT-OK-7731`, please ask directly in this chat.",
  "session_id": "20260911_12",
  "turn_id": "turn-35",
  "finish_reason": "stop",
  "tool_calls": 0
}
```

Two things about that contract are easy to get wrong:

- **An assistant message on the session bus is a streaming chunk.** The turn runner republishes every `AgentEvent`, and a streaming provider yields one `Message` per delta, each carrying the same provider id; the store only sees whole messages because `Conversation::push` folds same-id chunks together before they are persisted. `TurnFollower` folds them with that same function. Until it did, it kept the last non-blank chunk, so that refusal reached its caller as `Turn turn-35 finished (stop). Final message:\n\n.` — the reply's final token — and read as an ordinary completion (finding F3).
- **`declined` is a reading of prose, not a protocol.** Nothing the target is told was changed to make it — the untrusted-injection envelope is exactly as it was — so there is no marker to ask for. The reading is deliberately narrow: a first-person subject plus a refusal (*"I can't find any failing tests"* is a finding and is `completed`), or a passive *"has not been executed"* only in a sentence that also names the injection's trust framing. Anything subtler is `completed` with the whole `final_message` beside it, which is why the message is never dropped in favour of the verdict.

An `errored` turn is still an error result, and — exactly as while it travelled as a plain error — it neither reflects in the target's tab nor records the first-crossing pair; every other verdict does both, as the old success results did.

Both `steer` and `turn` post a toast on the target's tab naming the calling conversation. The toast is a notice, not the message — the message itself arrives in the transcript through the session bus (see above).

### When to reach for it

`note` to leave context a conversation should see next time it runs — and it is the recommended fallback wherever `turn` refuses. `steer` only when the target is mid-turn and going the wrong way. `turn` to make another conversation do work; add `wait: "final_message"` when you need the answer inline, and otherwise pair it with `workspace_watch`.

---

## `workspace_set_tools`

The only tool that changes what another conversation may use. Four independent dimensions: extensions, session-scoped skills, provider/model, knowledge bases.

### Arguments

| Name | Type | Default | Meaning |
|---|---|---|---|
| `session_id` | string | required | Target. |
| `add_extensions` | string[] | `[]` | Resolved against the config before anything is applied. |
| `remove_extensions` | string[] | `[]` | Gated and checked before anything is applied: a built-in capability is refused, and so is a name the conversation does not have loaded. |
| `add_skills` | string[] | `[]` | **Session-scoped only.** Never touches the machine-wide skill file. |
| `remove_skills` | string[] | `[]` | Same scope. |
| `provider` | string | — | Required whenever `model` is given. Legal on its own: `provider` with no `model` silently selects that provider's `metadata.default_model`, so the `model={provider}/{model}` label can name a model the caller never asked for. |
| `model` | string | — | Validated against the provider's `known_models`. |
| `set_knowledge_bases` | string[] | — | Replaces the session's set. `[]` clears it. Every name must be an installed base. |
| `primary_knowledge_base` | string | — | Three-valued: **absent** = `Auto` (keep the current target if it is still a member, else pin the first, else clear); `""` = `Clear`; a name = `Set`. Only meaningful with `set_knowledge_bases`, and a name outside that set is refused. |

### The pre-flight, then application

Every check that does not change anything runs first, in one function — `WorkspaceClient::preflight_set_tools` — so a bad request is a clean no-op. The same function is what the always-confirm inspector asks before it decides whether to raise a card ([below](#the-always-confirm-rule)), so a refusal reads the same whichever of the two produced it. In order:

- **Self-targeting** is refused: the tool changes *another* conversation.
- **The write gate** (`refuse_unless_writable`): a private target is out of reach of a public caller, with the one sentence it shares with an absent target.
- `add_extensions` goes through `get_extension_entry_by_name`, **not** `get_extension_by_name`. The difference is the operator's `enabled` flag: an extension an operator wrote `enabled: false` for is refused here with the same message `manage_extensions` gives, so this tool cannot be a second, ungated door around issue #42.
- Granting the `workspace` extension to a session whose type is `SubAgent` is refused outright: `subagent sessions can never be granted the workspace extension`. The match is on the *normalized resolved* config name, so `"Workspace"` — this extension's own configured name, and the spelling a model most often sends — does not slip past.
- `remove_extensions`, each name through Gate F1's unload decision (`extension_manager::manageability_refusal`, the function `assert_extension_manageable` itself asks): a built-in capability is refused with the extension manager's own sentence — *"`autovisualiser` is a built-in Biorouter capability, not an installed extension, and cannot be enabled or disabled through Extension Manager"* — then the tier and affiliation arms. The target's agent is **peeked, never created**; a conversation with no live agent is judged against nothing loaded, which is exactly what the handler's freshly built agent would answer. When the agent is live, a name it does not have loaded is refused too — see the retired false success below.
- `model` without `provider` is an error. An unknown provider lists the known ones. A model outside `known_models` is refused *unless* the provider publishes no catalog or declares `allows_unlisted_models` (ollama, llamacpp, gcpvertexai, custom providers). Under enforcement, a public provider for a private target is refused here by `privacy::bind_allowed`, Gate A's own predicate, rather than by `update_provider` after the change was approved.
- `set_knowledge_bases` requires the daemon, every name must be an installed base (when the host can enumerate them — `WorkspaceServices::installed_knowledge_bases`), and a named `primary_knowledge_base` must be one of them. This used to be checked last, after extensions, skills and the provider had already landed, and an unknown name was dropped from the set without a word.

Skills are **not** validated: a session-scoped skill override is a name, and the skill catalog is not a stable function of the machine (roots come and go with the working directory and with extensions), so a check against it would refuse legitimate grants.

Application then runs in order: extensions → skills → provider → knowledge bases. A live agent is fetched **only** when extensions or the provider change — `get_or_create_agent` is create-on-miss and its miss path caches a provider-less agent under the target's id, which a skills-only or KB-only call must not pay for.

> **Warning.** The pre-flight makes a bad *request* a no-op; it cannot make **application** atomic. A step that fails for a reason no pre-flight can see — a provider that cannot be constructed, an extension that fails to start — leaves the earlier steps applied.

> **Retired false success: removing an extension the target does not have.** `ExtensionManager::remove_extension` is a `HashMap::remove` on the normalized name that returns `Ok(())` whether or not anything was there, so a typo, or a removal from a session that never loaded that extension, used to come back as `-name` in the applied list, indistinguishable from a real removal. The pre-flight now refuses it — `` `x` is not enabled in conversation … `` — for a target whose agent is live, and only after both privacy arms, so a public caller naming a private connector still meets the one sentence for "loaded" and "not loaded" alike. For a target with **no** live agent the existence check does not run: the handler acts on a freshly built agent, which has nothing loaded, and `workspace_list` remains the way to see what that conversation holds.

### Returns

`Applied to session {id}: {labels}.` where labels are `+ext`, `-ext`, `+skill:name`, `-skill:name`, `model={provider}/{model}`, and one of `kb=<cleared>` or `kb=a+b (primary=c)`. A provider change appends ` The model change applies to this conversation's NEXT turn.` — a turn already running finishes on the provider it started with. Nothing requested returns `No changes requested for session {id}.`

The knowledge-base label reports what the **service stored**, not what was asked for, because the service may move the write target itself; echoing the request would teach the model a state the store does not hold.

Every applied change also posts a toast on the target tab listing the labels.

### The always-confirm rule

`WorkspaceMutationInspector` (`workspace_inspector.rs`) escalates to a confirmation card **with no mode gate at all** — including Fully Automatic — when the arguments contain any of:

- an `add_extensions` entry that is process-spawning (structurally, for `Stdio` and `InlinePython` configs; by name for the in-process `developer`, `computercontroller`, `code_execution`) or network-egress (`Sse`, `StreamableHttp`, which carry a URI plus credentials);
- a `remove_extensions` entry that is security-relevant (the list includes `workspace` itself and the extension manager) or that the operator authored explicitly in `config.yaml`;
- a `provider` switch, on the grounds that the target's whole stored history then goes to that endpoint;
- any `add_skills`, because a skill injects instructions into the target's prompt.

`remove_skills` and knowledge-base changes are **not** on that list. The same inspector also reads `workspace_open`'s `new.extensions`, because minting a conversation with the grant baked in is the easier route to the same capability.

**A card is only ever raised for a change that can be made.** Before it looks for a reason to confirm, the inspector runs the handler's own pre-flight ([above](#the-pre-flight-then-application)) through a transient client over the calling agent's session store, with the capability it sampled (or was pinned to). If the handler would refuse the call, the inspector returns `InspectionAction::Deny` instead of a card, and the agent loop hands the model that refusal verbatim (`Error: …`) rather than "the user has declined to run this tool" — nobody was asked. The 2026-09-10 QA run is why (finding F4): `remove_extensions: ["autovisualiser"]` raised *"This change removes 'autovisualiser', which the user configured explicitly"*, the user approved it, and only then did the handler answer that a built-in capability cannot be removed this way. The handler still re-runs the same pre-flight with the capability the call is admitted on, so the inspector's answer can only go stale in the fail-safe direction. This applies to `workspace_set_tools`; `workspace_open`'s card does not yet run a pre-flight.

### When to reach for it

When the user asks for a capability change in a conversation that is not the current one — the extension's instructions tell the model to do this itself rather than pointing at Settings, but only when it has the tool. There is no CLI counterpart: this is one of the two capabilities `workspace_parity.rs` declares `Counterpart::Asymmetry`, on the grounds that `biorouter extension` and `biorouter skill` are machine-wide rather than session-scoped.

---

## `workspace_close`

Three scopes of "stop", smallest blast radius first.

### Arguments

| Name | Type | Default | Meaning |
|---|---|---|---|
| `session_id` | string | required | Target. |
| `scope` | string | required | `tab` \| `turn` \| `agent`. Anything else is refused. |

### `scope: "tab"`

Sends a `close_tab` frame. With no GUI attached it is a **stated no-op**, not an error: `No GUI attached — nothing to close at tab scope (gui_attached: false).` The session and any running turn survive.

> **False success: a `close_tab` that the renderer refused.** The frame is sent with `wait_result: false`, so the renderer's answer is never read. The renderer refuses with `session has no tab` when the id has no tab open — and the tool still returns `Tab for session {id} closed (session survives).` Treat the success text as "the frame was dispatched", not "a tab closed". This is the same fire-and-forget shape as `subagent`'s tab announcement, below; unlike `workspace_open`, which does await its round trip.

### `scope: "turn"`

Requires the daemon (`scope:"turn" requires the BioRouter daemon`). Calls `services.cancel_turn`, which is idempotent: a session with nothing in flight returns `Session {id} had no turn in flight (nothing to cancel).` as a success, not an error. A real cancellation returns `Cancelled turn {turn_id} on session {id}.` and posts a toast naming the calling session.

### `scope: "agent"`

Requires the daemon. Calls `services.stop_agent` (cancel plus evict). The **session record is kept** — this stops the running instance, it does not delete the conversation. Returns `Agent for session {id} stopped and evicted (session record kept).` and posts a toast.

### When to reach for it

`turn` is the one to reach for to stop a runaway — including a background subagent, whose own start message recommends exactly `workspace_close { scope: "turn" }`. `agent` only when the instance itself needs to go (a wedged agent, a provider change that must not wait for the next turn). `tab` is cosmetic.

---

## `workspace_watch`

Parks until named conversations finish. This is the tool that replaces polling; the extension's instructions say so explicitly ("never poll workspace_read_conversation"). Read-only.

### Arguments

| Name | Type | Default | Meaning |
|---|---|---|---|
| `session_ids` | string[] | required | 1 to 32 ids. Empty is an error; more than 32 is an error naming the cap. |
| `mode` | string | `"any"` | `any` returns as soon as one finishes; `all` waits for every one. |
| `timeout_s` | u64 | `120` | `.clamp(1, 600)`, then shortened to fit the transport — see below. |
| `assume_running` | bool | `false` | Skip the liveness pre-check and park unconditionally. Use when a turn is known to be starting but may not have claimed its lock yet. |

### The effective wait depends on the transport (#110)

`timeout_s` is what the schema accepts; it is not always what the wait can be.
On a **bridged coding-agent turn** (`claude_code`, `codex`) the call is held open
inside the child's own MCP client, which applies a hard per-call wall clock and
abandons the request when it elapses. Issue #110 measured that at ~60 seconds
while this schema advertised 600, so every long watch failed with *"The operation
timed out"* — a transport failure, not the non-error partial report this handler
had ready.

Two things changed, and both are load-bearing:

- **Biorouter configures the child's deadline** (`bridge::CHILD_TOOL_CALL_TIMEOUT`,
  written as `timeout` in Claude Code's MCP config and `tool_timeout_sec` in
  Codex's), so a long watch fits.
- **The handler clamps anyway**, to `bridge::bridged_call_budget()` minus the room
  an answer needs. That is what keeps the guarantee when the configuration is not
  honoured — an older CLI, a renamed field, a user's own `MCP_TOOL_TIMEOUT`.

When the wait is shortened the reply says so, naming **both** numbers:

```text
(Waited 50s of the 600s requested: this turn's transport ends a single tool call
at 50s, so the wait was shortened to return this status instead of failing.
Watch again to keep waiting — the completions above are not repeated.)
```

Repeated bounded watches are therefore the intended pattern for multi-minute
work, and they lose nothing: every completion observed before the deadline is in
the report, including under `mode: "all"`, so the follow-up watch only names what
is still running. **A timeout is never an error**, on any transport.

### Cancelling a watch

`workspace_watch` is the only tool in this extension that *parks*, so it is the
only one the turn's cancel token has anything to reach. It honours it: Stop,
`AppState::cancel_turn`, a dropped websocket and a bridge lease dropping all end
the park at the instant they land, and the reply says it was cancelled rather
than letting "still running" read as the answer to a wait that never happened.
The watched conversations are untouched — what was cancelled is the turn doing
the watching.

⚠ Ending the park also **reaps the watcher tasks**, and that is not tidiness. Each
holds a `session_events::Subscription`, which only reclaims its session's
1024-slot event ring when it drops; before this they looped for the life of the
process after a watch ended, leaking a slot per watched session per watch. A wait
that used to be killed by a child's 60-second deadline can now legitimately park
for ten minutes, which makes those slots much easier to accumulate.

### How liveness is decided

Subscription happens **before** the pre-check, so a completion landing in the gap is not lost. Liveness is then three-valued, and the order is a veto rather than a fallback chain:

1. the background-subagent handle registry, scoped to the **calling** session (one conversation can never inspect another's children). A registered handle that `is_running()` means `Running`, full stop.
2. otherwise the daemon's `is_turn_active`.
3. otherwise `Unknown`.

The registry outranks the daemon because a background child is registered synchronously and only then queues on `SUBAGENT_SEMAPHORE` (default 8 concurrent). A queued child has no daemon turn lease, so a daemon-first check would report the ninth and tenth children of a fan-out as "already idle" before they had begun. Only a positive `Idle` short-circuits — `Unknown` parks, which is what keeps a headless process from reporting everything finished.

Terminal events are `TurnFinished` and `TurnError`; a lagged broadcast receiver keeps listening rather than giving up.

### Returns

Nothing finished:

```text
No conversation finished within 120s. Still running: a, b. They keep running — watch again or read them later.
```

plus, when any id resolved as `Unknown`, a second line saying no daemon is attached so whether they had started could not be checked — "some of these may never have been running". That distinction between *still running* and *we could not tell* is deliberate; do not read the first form as evidence of work in progress.

Something finished:

```text
Completed:
- {id} ({reason})
Still running: {ids}

Read a completed conversation with workspace_read_conversation (view:"summary" for its outcome, view:"tool_calls" for what it did).
```

A timeout is not an error in either form.

### When to reach for it

Immediately after `subagent { background: true }` or after `workspace_send_prompt { mode: "turn" }` without a wait — those two tools' own result text names it. `mode: "all"` for a fan-out you need complete; `mode: "any"` to react to the first result.

---

## `workspace_open`

Opens or focuses a conversation, and is the only tool that creates one.

### Arguments

| Name | Type | Default | Meaning |
|---|---|---|---|
| `session_id` | string | — | Open/focus an existing conversation. Mutually exclusive with `new`. |
| `new` | object | — | Create one. Mutually exclusive with `session_id`. |
| `placement` | string | `"tab"` | `tab` \| `split` \| `window`. Closed vocabulary, validated **before** anything is created. |
| `focus` | bool | `false` | Defaults to false so a new tab never steals the composer. |

`new` carries `kind` (**required**), `working_dir`, `extensions`, `knowledge_bases`, `primary_knowledge_base`, `prompt`. Passing both `session_id` and `new` is an error (`pass either session_id OR new, not both`); passing neither is an error too.

### `new.kind`, and why this tool cannot delegate

`new.kind` is a required, closed vocabulary in the **same** spelling `workspace_list` reports as `session_type` — a conversation's kind has one set of names in the system, not one per tool:

| Value | Result |
|---|---|
| `"user"` | Creates a conversation the **user** owns: `session_type: user`, no parent. This is the only kind this tool creates. |
| `"sub_agent"` | **Refused**, with a result that names the `subagent` tool, says what only that tool can do (stamp this conversation as the child's parent before its first turn, apply the subagent restrictions and lifecycle), and tells the caller to pass `kind:"user"` if a peer conversation was actually meant. |
| anything else — `scheduled`, `hidden`, `terminal`, a typo, or absent | Refused, stating the two values above. |

This exists because of [#111](https://github.com/BaranziniLab/biorouter/issues/111). `workspace_open { new: { prompt } }` and `subagent` both read as "start a conversation and give it a first instruction", so an explicit request to spin up three sub-agents produced three ordinary `user` rows with a null `parent_session_id` — sessions History's nesting could never show and `workspace_list { only_subagents }` could never find. The work ran; the sessions were not subagents in the data model.

The fix is a **declaration, not an inference**. Nothing is read from the prompt: a conversation the user owns may legitimately open with a first prompt, so a heuristic on the prompt would misclassify exactly the conversations that matter most. And nothing is reclassified retroactively — an existing unparented `user` session stays one whatever its title looks like.

The check runs as the **first statement of `open_new_session`**, ahead of the extension gate, the daemon lookup and `create_session`. A refusal that had already minted a row would produce the exact outcome the refusal exists to prevent.

### Creating

- Requires the daemon: `starting a new session requires the BioRouter daemon`.
- `working_dir` **defaults to the caller's**. A different directory is allowed but never silent — it is named in the tool result *and* in a toast, and the toast is deliberately emitted **after** placement, because a renderer that routes a session's toasts to that session's tab would drop one that arrived before the tab existed.
- `primary_knowledge_base` absent means `Auto`, which on a brand-new session pins the first id. `workspace_open` always chooses one rather than leaving a session with bases and no write target, in which KB-less writes fail.
- `prompt` runs as a detached turn, provenance-stamped as an agent injection from the caller, and it **spends a §5 fan-out slot** — the same per-caller budget `workspace_send_prompt` spends, not a second one. Over budget the whole call is refused with `this session already has N injected turns in flight (cap M); wait for one to finish before opening another conversation with a prompt`.
  - The cap is checked **before `start_session`**, for the same reason the `new.kind` refusal above is: a refusal that has already minted a row produces the unparented conversation the refusal exists to prevent.
  - The budget is shared because two tools start a detached turn in someone else's conversation, and for a while only one of them counted. A model that reached for `workspace_open` instead of `workspace_send_prompt` therefore fanned out with nothing counting at all — which surfaced as a single request opening several side conversations that all ran the same task.
  - **Creating a conversation is not what is capped.** The same call without a `prompt` still succeeds at the cap; only the injected turn is budgeted.
- It **never** retargets an existing session's working directory; the directory is set at creation, so it takes neither post-creation writer and cannot race the turn guard those share.

Opening an existing session validates that it exists first (`no such session: …`), so a dangling frame never reaches the GUI.

### Placement and the renderer round trip

`placement: "window"` sends its own `open_window` frame; `tab` and `split` ride `open_tab` carrying `placement` and `focus`. The frame is sent with `wait_result: true` and the renderer's `workspace_result` is read, so this tool reports honestly:

- `Session {id} opened in the GUI (tab, background). {detail}` when the renderer answered `ok: true`;
- `Session {id} NOT opened in the GUI (split, background). split refused: already at 6 groups` when it refused. The pane ceiling is `MAX_GROUPS = 6` in `ui/desktop/src/components/chatGroups/chatGroupsLayout.ts`, and the planner's refusal uses the same `groupCountOf` predicate the reducer itself uses, so the two cannot disagree.

An absent or non-boolean `ok` counts as a refusal — an unparseable answer is not evidence.

> **Caveat on `window`.** The planner returns `{ ok: true, detail: 'window requested' }` and hands the id to the create-chat-window IPC without waiting for it. A window that then fails to appear is still reported as requested.

If the GUI call fails outright *after* a session was created, both halves are reported: `Session {id} was created but the GUI did not place it ({e}). … It exists and can be reached with workspace_send_prompt — do NOT create another.` That message exists so a model does not respond to a placement failure by minting an orphan.

Headless: `Session {id} ready (gui_attached: false — no tab opened; the session exists headlessly).`

### With announce-only on

`WORKSPACE_ANNOUNCE_ONLY` (Settings → App → Workspace → "Never open tabs automatically", default **off**) downgrades `open_tab`, `open_window` and `activate_tab` to a `notify` frame. The model-facing text changes with it: it names the right noun (window vs tab), says no such thing was opened, and instructs the model not to claim otherwise. If the *notification* is itself refused by the renderer, the text says the conversation is waiting in History rather than that the user was told.

### When to reach for it

To start parallel work **the user** should be able to see, and to bring an existing conversation forward. Not to delegate: delegation is `subagent`, whatever the request is phrased as. Note that there is no separate "focus" call — `workspace_open { session_id }` always sends `open_tab` and relies on the reducer's dedupe/adopt rule to focus an existing tab.

---

## `subagent`

The one spawn tool, advertised by this extension under its pre-existing bare name (`SUBAGENT_TOOL_NAME = "subagent"`).

> **Advertised here, dispatched elsewhere.** The agent loop intercepts the call before it reaches the extension (`is_spawn_tool_call` in `agent.rs`), because dispatch needs the parent's `TaskConfig` — provider, extensions, working dir — which only `Agent::dispatch_tool_call` holds. The extension's own `call_tool` arm exists but returns `` `subagent` is dispatched by the agent loop, not by this extension ``, and is reachable only if that interception is ever removed. If you see that string in a transcript, the interception is broken.

This is also why `subagent` works in a terminal `biorouter session` with no `workspace` extension enabled: `Agent::reply` → `prepare_tools_and_prompt` → `list_tools` → `ensure_spawn_extension` loads a restricted delegation surface for any session where delegation is permitted. That surface includes `subagent` plus `workspace_watch`, `workspace_read_conversation`, and `workspace_close`, so the parent can monitor, collect, and stop its own child. It does not include broad listing, steering, tool mutation, or session creation.

### Arguments

| Name | Type | Default | Meaning |
|---|---|---|---|
| `instructions` | string | — | Ad-hoc task. Required unless `subworkflow` is given. |
| `subworkflow` | string | — | Name of a predefined subworkflow. |
| `parameters` | object | — | Only valid with `subworkflow`. |
| `extensions` | string[] | — | Omit to inherit all; `[]` for none. |
| `settings` | object | — | `{ provider, model, temperature }` overrides. |
| `summary` | bool | `true` | Return only the child's final summary. |
| `visible` | bool | — | Show the child in its own tab. Defaults to true when a GUI is attached. `false` forces a silent run. |
| `placement` | string | `"tab"` | `tab` \| `split` \| `window`, validated before a session is created. |
| `background` | bool | `false` | **Advertised only when `background_enabled()`** — the config parameter `BIOROUTER_SUBAGENT_BACKGROUND`. Passing it while the flag is off is silently ignored and the call blocks as usual. |

### Concurrency limits

Three separate bounds, all env-tunable:

| Limit | Default | Variable | Effect at the limit |
|---|---|---|---|
| Total in flight (queued + running) | 64 | `BIOROUTER_SUBAGENT_MAX_INFLIGHT` | The spawn is **refused** with `Subagent limit reached: …` |
| Concurrently running | 8 | `BIOROUTER_SUBAGENT_MAX_CONCURRENT` | The spawn **queues** on a semaphore |
| Visible tabs per parent | 4 | `BIOROUTER_WORKSPACE_MAX_VISIBLE_CHILD_TABS` | The spawn is **downgraded to background**, never refused |

The tab cap is claimed and checked under one lock (`VisibleChildGuard::try_claim`), because subagent dispatch is deliberately excluded from the tool-dispatch semaphore and a parallel fan-out would otherwise all read zero and all claim. The slot is released when the child's run ends, so a parent that spawns four, waits, and spawns four more gets tabs both times — it bounds the burst, not the total.

A downgraded child is told to the parent through `ChildVisibility::parent_note`, appended to the tool result: `Subagent {id} is running in the background: you already have 4 subagent tabs open, which is the limit. It is listed in History under this conversation and you can read it with workspace_read_conversation.` The announce-only case gets its own note in the same place. `visible: false` and headless get no note — those are what the caller asked for or already knows.

### Returns

Blocking (the default): the subagent result envelope — the child's final summary when `summary: true` — with the visibility note appended when there is one.

Background: an immediate handle message naming both ids and the three follow-ups, plus `structured_content` carrying the handle snapshot:

```text
Subagent started in the background (handle `…`, session `…`). It keeps working while you do.
- Wait for it: workspace_watch {"session_ids": ["…"]}
- Check on it: workspace_read_conversation {"session_id": "…", "view": "summary"}
- Stop it: workspace_close {"session_id": "…", "scope": "turn"}
```

Every workspace tool takes the child's **session id**, not the registry handle id.

### False success: a tab announcement the renderer refused

> **Warning — this is a real, documented-in-source honesty gap.** `announce_open_frame` sends the child's `open_tab` / `open_window` frame with `wait_result: false`. A renderer refusal — `split refused: already at 6 groups`, or any other failure — is discarded, while the caller has already been handed `ChildVisibility::Visible`, whose `parent_note` is empty. In that narrow case the model believes a tab opened when none did, and the visible-tab slot stays claimed for the child's whole run.

The trade-off is deliberate and stated in `subagent_tool.rs`: awaiting the round trip would couple every spawn to the renderer's 10 s `emit_and_wait`, so one wedged window would stall an entire fan-out, and a lost spawn costs far more than a misplaced tab. `workspace_open` does not have this gap because `place_in_gui` can afford to park on the answer. The consequence is bounded — the child exists, runs, is badged in History and is readable with `workspace_read_conversation` wherever its tab did or did not land — but the extension guide's sentence "the parent is told which children did not get a tab" is true for the **cap** path and not for the **renderer-refusal** path.

A second, smaller point on the same path: the `annotate_tab` badge frame **is** sent for every announced child including a capped one, deliberately, so the badge is already waiting when the user opens that tab later from History. Any claim that `annotate_tab` has no emitter is stale — `subagent_tool.rs` emits it and `ChatGroupsContext.tsx` consumes it into `tabAnnotations`.

### When to reach for it

Delegation with a fresh context window. The parent receives only the final summary, which is why the instruction block pairs it with `workspace_read_conversation view: "tool_calls"` to verify what the child really did. For parallel work, issue several `subagent` calls in the same assistant message.

---

## Frames, and one that is dead

The renderer accepts **six** GUI commands, and `WorkspaceCommand.cmd` in `workspaceCommandRegistry.ts` is the closed union that names them: `open_tab`, `activate_tab`, `close_tab`, `open_window`, `notify`, `annotate_tab`. `workspaceCommandPlanner.ts` switches on exactly those six; its `default` arm refuses anything else with `unknown cmd '…'`, which is the planner's answer to an unrecognised frame, not a seventh command.

> **`activate_tab` has no production emitter.** Its only occurrences in `crates/` are the `FOCUS_STEALING_CMDS` list it belongs to, a doc comment, and one unit test. `workspace_open` always sends `open_tab` and relies on the reducer's dedupe rule to focus an existing session, so nothing constructs an `activate_tab` frame. The renderer's handler is live and correct; it is simply never reached today. Treat it as reserved vocabulary, not as a capability — and do not document a "focus an existing tab" frame as something an agent can trigger.

Two names are retired and pinned as such by `RETIRED_TOOL_NAMES`: `subagent_status` and `workspace_spawn_subagent`. Neither is a live tool; documentation or a prompt that mentions either is describing a surface that no longer exists. `subagent_status`'s list mode became `workspace_list`'s `parent_session_id` / `only_subagents` filters, and its wait mode became `workspace_watch`.

`PENDING_TOOLS` is empty, which asserts the stronger claim that no tool is currently advertised ahead of its handler.

## What could not be verified from source

- **Packaged-app behaviour.** Every GUI claim on this page is read from the daemon-side frame construction and the renderer's pure planner. Whether the signed and notarized desktop build has ever exercised the bridge end to end is not something source can answer, and the design of record says only the dev build was verified. Nothing here should be read as a statement about the packaged app.
- **`gui_command`'s delivery guarantees** beyond the `wait_result` flag — the false-success findings above are derived from that flag and the planner's refusal paths, not from an observed failure.

## Related documentation

- [Session metadata contract](session-metadata-contract.md) — the ID, kind, parent and subagent-run identity every one of these tools resolves against.
- [Workspace control](workspace-control.md) — the task-oriented guide: laying work out across tabs, panes and windows, and the caps you meet in practice.
- [Workspace Control capability](../extensions/built-in/workspace.md) — the user-facing guide: the two tiers, how to enable the full surface, the confirmation card, focus etiquette, and the CLI capability table.
- [Subagents](subagents.md) — the glass-box tab, `human_intervened`, and what closing a child's tab does and does not do.
- [Tool routing](tool-routing.md) — which of these tools the model should prefer, and the disambiguation against Chat Recall, Memory and the knowledge base.
- [Agent workspace control (BR-71 design)](designs/agent-workspace-control.md) — the design of record, including the §4.3 frame vocabulary and the §5 permissions and abuse analysis.
- [Permission modes](../security/permission-modes.md) — the modes `mode: "turn"`'s headless refusal and the always-confirm inspector are graded against.
- [CLI command reference](../cli/command-reference.md) — the `biorouter session` subcommands that are these tools' counterparts.
