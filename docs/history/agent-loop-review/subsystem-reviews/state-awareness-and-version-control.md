# State, awareness, todos, goals and version control — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It documents how the agent knows where it is and what it is doing — working-directory propagation, the todo blob, `/goal` state, cross-session memory, mistake signals, and the SQLite session schema — and records ten gaps.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, and the review whose findings the current code contradicts most. Gap #1 (no repo map) was fixed by BR-1 (`agents/workspace_summary.rs`), gap #2 (which this review calls "the single biggest gap" — no version control of the agent's own edits) by BR-43 (`crates/biorouter/src/checkpoint/`), gap #4 (the full-overwrite todo blob) by the structured-todo proposal, gaps #5 and #6 (three disjoint memory stores) by BR-17, and gap #9 (large payloads stored inline in `content_json`) by BR-7. Every "absence finding" below should be read as "absent at review time".
> **Audience:** developers working on session state, memory, checkpoints, or workspace awareness.

Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); the numbered items under "Gaps and weaknesses" are what sibling reviews and the [review README](../README.md) cite as `state-awareness.md gap #N` (the file's former name).

## Scope and files reviewed

The subsystem question is **how the agent knows where it is, what it is doing, what it has done, and whether it is going wrong.** All paths below are repository-relative.

> **Warning.** The review recorded its reviewed state as branch `ui-hardening-a11y-tests`, which is unrelated to the rest of the corpus and disagrees with the only commit-pinning sibling ([guardrails and permissions](guardrails-and-permissions.md), at `24cdc3a2` on `main`). The review corpus therefore contradicts itself about what was reviewed. Treat line numbers as pointers to the right function, not exact locations.

## Overview

The agent's situational state is assembled from four largely-independent
mechanisms, none of which resemble the "repo map + checkpoint + diagnostics"
stack of a modern coding agent:

1. **Working-directory propagation** — a single `PathBuf` on each `Session`
   (`session_manager.rs:70`), pushed into the `ExtensionManager`
   (`extension_manager.rs:495`) so child-process MCP servers spawn with the right
   cwd, and surfaced to the model only as one line inside a per-turn "MOIM"
   message.

2. **MOIM (`<info-msg>`) re-injection** — every provider call, `inject_moim`
   (`moim.rs:12`) splices a fresh user-role message containing the current time,
   the working dir, and each *platform* extension's `get_moim()` output. The todo
   extension's `get_moim` (`todo_extension.rs:197`) is what re-surfaces the task
   list; that is the primary "keep the agent on track" channel.

3. **Persistent state** lives in one SQLite DB (`session_manager.rs:1054`):
   a `sessions` row (working_dir, tokens, workflow, `extension_data` JSON blob) +
   an append-only `messages` table (role, `content_json`, timestamps, metadata).
   Todos and "enabled extensions" are stored as versioned keys inside the
   session's `extension_data` blob (`extension_data.rs:28`), not as first-class
   columns.

4. **Cross-session memory** — three separate paths: `chatrecall` (SQL LIKE
   search over past messages, `chat_history_search.rs`), the Knowledge base
   subsystem (a real git-backed markdown wiki in `biorouter-mcp`), and
   `platform__ingest_conversation` (fold a chat into a KB, `knowledge_tool.rs`).

Text data-flow for one turn:

```text
user msg ──► add_message() ──► SQLite messages table
                                     │
agent loop iteration:                ▼
  load conversation ──► inject_moim(session_id, conv, ext_mgr, working_dir)
        │                     │
        │                     ├─ "<info-msg> It is <time> / Working directory: <wd>"
        │                     ├─ todo.get_moim()  → "Current tasks and notes:\n<todo content>"
        │                     └─ inserted as user message after last assistant
        ▼
  stream_response_from_provider(system_prompt, conv+moim, tools)
        │
  tool calls ──► CallToolResult (is_error flag) ──► fed back as tool_response
        │
  Stop hook (goal judge, optional) ──► block/allow finishing the turn
```

There is **no repo map, no file tree in the prompt, no post-edit diagnostics,
and no git checkpointing of the agent's own edits.** Those absences are detailed
below.

## Review questions answered

### How the agent understands its surroundings

**Working directory propagation.** Each `Session` carries `working_dir: PathBuf`
(`session_manager.rs:70`), stored as a `TEXT NOT NULL` column
(`session_manager.rs:1079`). At turn start the agent binds it into the extension
manager: `extension_manager.set_working_dir(session.working_dir.clone())`
(`agent.rs:1044`). `ExtensionManager` keeps it in a `Mutex<Option<PathBuf>>`
(`extension_manager.rs:111`) and `resolve_working_dir()` prefers it, falling back
to the process cwd (`extension_manager.rs:502-507`). Child-process MCP servers
are then spawned with `command.current_dir(dir)` **and** `BIOROUTER_WORKING_DIR`
env var (`extension_manager.rs:246-251`), with an env-var fallback if no dir was
passed (`extension_manager.rs:240-244`). This is the mechanism that makes the
GUI folder-picker actually change where the shell tool runs.

**What the model is told about its surroundings** is minimal: a single line in
the per-turn MOIM: `"Working directory: {}"` (`extension_manager.rs:1490-1493`).
That is the *entire* project-location signal. The system prompt template
(`prompts/system.md`) contains **no working directory, no file listing, no
directory tree**.

**Project-structure awareness ≈ none.** The only project-context mechanism is
hint-file loading: `PromptManager::with_hints` (`prompt_manager.rs:72-97`) reads
`.biorouterhints` / `AGENTS.md` files (names from `CONTEXT_FILE_NAMES`) found in
the working dir, honoring gitignore, and appends their text to the system prompt.
This is user-authored guidance, **not** a generated repo map. There is **no
equivalent of Aider's repo-map, Claude Code's file tree, or an automatic
`ls`/glob of the workspace injected into context.** The agent discovers structure
only by actively calling shell/`text_editor view`/glob tools.

> Absence finding: no code anywhere generates a file listing or symbol map for
> the prompt. Grepping `repo`, `file_list`, `list_files`, `project structure`
> across `agent.rs`/`prompt_manager.rs` returns nothing relevant.

### How the agent tracks what it is working on — todos and goals

**Todo extension** (`todo_extension.rs`). A platform MCP server exposing one tool,
`todo_write`, which **overwrites the entire todo content** (`todo_extension.rs:132-154`;
the description literally warns "completely replaces the existing content").
Content is capped at `BIOROUTER_TODO_MAX_CHARS` (default 50 000,
`todo_extension.rs:87-97`). On write it loads the session, stores a `TodoState`
into `session.extension_data` under key `todo.v0` (`todo_extension.rs:102-111`,
`extension_data.rs:87-90`), and persists via the session update builder.

Re-injection is the clever part: `TodoClient::get_moim` (`todo_extension.rs:197-214`)
reads the persisted `TodoState` and returns `"Current tasks and notes:\n<content>"`;
if empty it returns a nudge to "immediately update your todo with all explicit
and implicit requirements". `collect_moim` (`extension_manager.rs:1509-1516`)
gathers every platform extension's `get_moim` and folds it into the `<info-msg>`,
which `inject_moim` splices into the conversation on **every** provider call
(`agent.rs:1596`). So the todo is durable (survives compaction, since it lives in
`extension_data`, not the message log) and re-presented each turn. This is the
strongest state mechanism here.

**Goals** (`goal.rs`). `/goal <condition>` is a Claude-Code-style "keep working
until verifiably done" loop. It installs a **Stop-hook** LLM judge
(`goal.rs:246-265`) that evaluates the condition against a truncated
`transcript_tail` (`goal.rs:199-227`) every time the agent tries to finish. If
unmet, the stop is blocked and the judge's feedback is fed back
(`GoalOutcome::Continue`). Robustness features: an iteration cap that does *not*
reset on tool calls (`GOAL_MAX_ITERATIONS = 20`, `goal.rs:53`, `record_goal_block`
at `goal.rs:293-331`), Jaccard-similarity stall detection (`GOAL_STALL_LIMIT = 3`,
`reason_similarity` at `goal.rs:121-133`), and graceful give-up with a
best-effort-answer instruction (`giveup_instruction`, `goal.rs:182-195`).

Persistence contrast worth noting: **goal state is in-memory only** —
`GoalRegistry { goals: Mutex<HashMap<String, GoalState>> }` (`goal.rs:99-101`),
keyed by session id, held on the `Agent`. Unlike todos, a goal does **not**
survive a daemon restart. Todos persist to SQLite; goals do not.

### Version control of the agent's own edits

**No git checkpointing, no shadow git, no session-level undo of edits.** This is
a clear absence.

- `git2` is a dependency of exactly one crate for exactly one purpose: the
  **Knowledge base** wiki history (`biorouter-mcp/Cargo.toml:77`, used only in
  `biorouter-mcp/src/knowledge/git.rs`). It versions markdown pages in
  `~/.config/biorouter/knowledge/<kb>/.git`, **not** the user's project files or
  the agent's code edits.

- The only edit-undo that exists is the `text_editor` tool's **in-memory,
  per-process** history: `file_history: Arc<Mutex<HashMap<PathBuf, Vec<String>>>>`
  created fresh in `DeveloperServer::new()`
  (`developer/rmcp_developer.rs:698`). Before each mutating op, `save_file_history`
  pushes the *entire current file content* as a `String` onto a per-path stack
  (`text_editor.rs:1086-1106`); `undo_edit` pops one and writes it back
  (`text_editor.rs:1052-1084`).

  Limitations of this "undo":
  - It is a bounded LIFO of whole-file strings held in RAM; it dies with the
    developer server process and is **never persisted**.
  - It only covers files touched via the `text_editor` tool. Files changed by
    `shell` redirects, `write_file`, or any other extension are invisible to it.
  - There is no cross-file atomic rollback, no "checkpoint before this turn", no
    "revert the whole task". Each `undo_edit` reverts one file, one step.
  - `RunState` (`guardrails/run_state.rs`) is sometimes described with the word
    "snapshot", but it is a **paused-run approval token** (session handle +
    pending tool + remaining turn budget, `run_state.rs:49-67`), *not* a
    filesystem or conversation checkpoint. The doc-comment explicitly says the
    conversation is not embedded and cross-process resume is out of scope
    (`run_state.rs:9-18`).

> Absence finding: modern coding agents (Claude Code, Cursor, Aider) create a
> checkpoint (git stash / shadow commit) before edits so a whole task can be
> reverted. BioRouter has nothing equivalent. The best a user can do is
> `undo_edit` file-by-file, in-memory, only for `text_editor` edits, only within
> the life of one developer-server process.

### How cross-session memory works

Three unrelated mechanisms:

**a) Chat Recall** (`chatrecall_extension.rs` + `chat_history_search.rs`). A
read-only tool with two modes (`chatrecall_extension.rs:63-71`):
- *search*: keyword LIKE search across all sessions' messages. `ChatHistorySearch`
  splits the query on whitespace and builds `LOWER(json_extract(value,'$.text'))
  LIKE %word%` clauses joined by `OR` (`chat_history_search.rs:117-172`), joined
  to `sessions`, ordered by recency, excluding the current session
  (`chatrecall_extension.rs:187-188`). It is a substring OR-match, **not**
  semantic search, **not** FTS5 — no ranking beyond timestamp.
- *load*: given a `session_id`, returns the first 3 + last 3 messages
  (`chatrecall_extension.rs:121-154`).

**b) Knowledge bases** — the git-backed markdown wiki (`biorouter-mcp/knowledge/`,
including a built-in "Soul" base of durable user facts referenced in the system
prompt, `prompts/system.md:31-36`). The agent is instructed to consult relevant
KBs (including Soul) when a request may depend on user/project knowledge. This is
the closest thing to durable long-term memory, but it is **opt-in per query** —
nothing auto-injects Soul facts into every turn.

**c) Conversation ingest** (`platform__ingest_conversation`,
`platform_tools.rs:9-51`, handler `knowledge_tool.rs:24-90`). Lets the user say
"remember this chat"; it loads the requested sessions, renders the transcript to
markdown, and runs the KB ingestion pipeline (`knowledge_tool.rs:63-79`). Target
KB resolution is explicit-id → new-by-name → active (`knowledge_tool.rs:94-127`).

> The three do not share an index. Chat Recall never touches the KB; the KB never
> indexes raw chat unless the user explicitly ingests. There is no unified
> "memory" store.

### How the agent knows when it is making a mistake

Feedback signals are thin and almost entirely **tool-result-driven**:

- **Tool errors** are the main signal. `CallToolResult::error(...)` is returned
  by extensions (e.g. `todo_extension.rs:186-189`, `chatrecall_extension.rs:309-312`)
  and dispatch failures are wrapped with `is_error: Some(true)`
  (`tool_execution.rs:116`, `:214`) and fed back to the model as the tool
  response. The model must read the error text and self-correct. There is no
  structured error taxonomy.

- **`diagnostics.rs` is NOT code diagnostics.** Despite the name,
  `session/diagnostics.rs` collects *system/support* info — app version, OS,
  enabled extensions (`diagnostics.rs:25-44`) — and zips logs + session export
  for a bug report (`generate_diagnostics`, `diagnostics.rs:72-137`). It has
  nothing to do with detecting agent mistakes.

- **No LSP / compiler / lint feedback loop.** Grepping the developer extension
  for `lint`, `diagnostic`, `LSP`, `cargo check`, `type check` returns nothing in
  non-test code. After an edit, the agent gets **no** automatic compile/lint
  result; it only learns of breakage if it *chooses* to run the build/tests
  (which `prompts/system.md:63-64` merely *encourages*: "run the project's build,
  tests, or lints when available"). An `LSP` tool exists in the harness's
  deferred-tool list but is not part of the core edit→feedback loop.

- **Goal judge** (`goal.rs`) is the one active "are you actually done?" signal,
  but it fires only when a `/goal` is set and only at turn-end, judging a
  transcript tail — it catches "didn't finish", not "wrote a bug".

- **Loop/stall detection** exists only inside the goal loop (stall similarity,
  `goal.rs:303-311`) and a generic Stop-hook block cap (`STOP_HOOK_BLOCK_CAP`,
  referenced `goal.rs:20-22`). There is no general "you called the same failing
  tool 5 times" guard in the main loop.

### How session state persists — the SQLite schema

One SQLite DB per data dir, schema built in `create_schema`
(`session_manager.rs:1054-1165`), version-tracked in a `schema_version` table
(`session_manager.rs:1057-1069`) with numbered migrations
(`run_migrations`/`apply_migration`, `session_manager.rs:1271-1509`).

**`sessions` table** (`session_manager.rs:1073-1097`): `id` PK, `name`,
`description`, `user_set_name`, `session_type`, `working_dir`, `created_at`,
`updated_at`, `extension_data` (JSON, default `'{}'`), six token columns
(current-turn `i32` + accumulated `i64`, see the overflow note at
`session_manager.rs:85-91`), `schedule_id`, `workflow_json`,
`user_workflow_values_json`, `provider_name`, `model_config_json`,
`diverged_from`, `external_key`.

**`messages` table** (`session_manager.rs:1104-1113`): `id` autoincrement,
`session_id` FK, `role`, `content_json` (the serialized `Vec<MessageContent>` —
text, tool requests, tool responses, thinking), `created_timestamp`, `timestamp`,
`tokens`, `metadata_json`. Insert is append-only (`add_message`,
`session_manager.rs:1843-1870`) and also bumps `sessions.updated_at`. Load
reconstructs `Message`s ordered by timestamp and re-derives synthetic ids
`msg_<session>_<idx>` (`get_conversation`, `session_manager.rs:1810-1841`).

**`token_events` table** (`session_manager.rs:1123-1134`): append-only per-turn
token accounting, added as a side table (migration 10) to avoid a lost-update
race on the sessions row (`session_manager.rs:1119-1121`, `TokenDelta` at
`session_manager.rs:106-118`).

**Extension state** (todos, enabled-extensions) is **not** a column — it is
JSON inside `sessions.extension_data`, keyed `"<name>.<version>"` via
`ExtensionData::set_extension_state` (`extension_data.rs:28-37`). `TodoState`
uses key `todo.v0` (`extension_data.rs:87-90`).

Per-message, therefore, what is stored is: role, full content JSON (including
every tool call + response), two timestamps, optional token count, optional
metadata JSON. Whole conversations can be branched (`diverged_from`) and replaced
wholesale (`replace_conversation_inner` deletes + re-inserts,
`session_manager.rs:1872-1904`) — which is how compaction/edits rewrite history.

## Notable design choices (worth keeping)

- **MOIM re-injection of durable state each turn** (`moim.rs:12`,
  `extension_manager.rs:1509-1516`). Persisting todos in `extension_data` and
  re-presenting them every provider call means task state survives compaction and
  is always in the model's fresh context window — a genuinely good pattern that
  most agents approximate with fragile "reminder" system-prompt hacks. The
  minute-granularity timestamp (`extension_manager.rs:1488`) to avoid busting the
  prompt cache every second is a nice touch.

- **MOIM insertion-point safety** (`moim.rs:31-41`): it inserts after the last
  assistant message *and* any trailing tool responses, specifically to avoid
  putting a user message between a tool_call and its tool_result (which OpenAI
  rejects). This is a subtle, correct piece of protocol handling.

- **Goal loop robustness** (`goal.rs`): the truncation-aware judge rule
  (`goal.rs:139-178`), a real non-resetting iteration cap, stall detection, and
  graceful give-up together address a real failure mode (infinite loops on
  "summarize 400 sites in chat"). Well-reasoned and well-tested.

- **Append-only token side-table** (`session_manager.rs:1119-1134`) to dodge a
  lost-update race — correct concurrency thinking.

- **`extension_data` versioned-key scheme** (`extension_data.rs`) is a clean,
  forward-compatible way to bolt per-extension session state on without schema
  churn.

## Gaps and weaknesses

These ten items fed the improvement phase. They are what other documents in this
review cite as `state-awareness.md gap #N`; the numbering below is that scheme and is stable.

1. **No repo map / workspace awareness.** The model gets one line ("Working
   directory: …") and nothing else about project structure
   (`extension_manager.rs:1490`). State-of-the-art coding agents inject a repo
   map / file tree / symbol index. BioRouter forces the model to spend tool calls
   rediscovering structure every session. High-value, low-risk addition: a
   cached, gitignore-aware directory/symbol summary in the MOIM or system prompt.

2. **No checkpoint / shadow-git for agent edits.** The only rollback is
   `text_editor`'s in-memory, per-file, per-process LIFO (`text_editor.rs:1052-1106`).
   There is no "revert this whole task", no snapshot before a risky turn, no
   coverage of shell-driven or non-`text_editor` writes. `git2` exists in-tree
   (for the KB) but is never applied to the workspace. This is the single biggest
   gap vs. Claude Code / Cursor / Aider, all of which checkpoint before editing.

3. **Goals are in-memory only** (`goal.rs:99-101`). A daemon restart silently
   drops an active goal, while todos persist — an inconsistency that will confuse
   users. Goals should persist to `extension_data` like todos.

4. **Todo is a full-overwrite blob, not structured** (`todo_extension.rs:132-154`).
   `content: String`, replace-everything semantics. No per-item state, no
   completion tracking the app can render, no diff. The "WARNING: completely
   replaces" caveat pushes correctness onto the model — a classic source of
   accidental task-list truncation. A structured todo (list of items with status)
   would be more robust and UI-friendly.

5. **Chat Recall is substring LIKE search, not semantic or FTS**
   (`chat_history_search.rs:117-172`). OR-matched `%word%` over JSON-extracted
   text, ranked only by recency. It will miss paraphrases and rank poorly on
   large histories. SQLite FTS5 (already available) or embeddings would be a
   large quality win; the schema even lacks an FTS index.

6. **No unified memory.** Chat Recall, Knowledge bases, and conversation-ingest
   are three disjoint stores with no shared index and no automatic promotion of
   useful facts. "Soul" is opt-in per query, never auto-injected. Compare to
   agents that maintain a single, always-consulted memory.

7. **Weak mistake-detection loop.** The only automatic error signal is a tool's
   own `is_error` text (`tool_execution.rs:116`). There is **no** post-edit
   compile/lint/LSP feedback (`session/diagnostics.rs` is support-bundle
   plumbing, misleadingly named). The system prompt only *asks* the model to run
   tests. A PostToolUse "run diagnostics after edit" hook feeding results back
   would materially improve correctness.

8. **No general loop-guard in the main agent loop.** Repeated identical failing
   tool calls are only caught inside the optional goal loop
   (`goal.rs:303-311`). Outside a `/goal`, an agent can retry the same failing
   action indefinitely with no systemic circuit-breaker.

9. **`content_json` stores everything inline, unbounded.** Large tool responses
   are serialized whole into `messages.content_json` (`session_manager.rs:1857`).
   No externalization/large-blob table beyond the runtime `large_response_handler`;
   session DBs can bloat, and message load deserializes all of it eagerly
   (`get_conversation`, `session_manager.rs:1810-1841`).

10. **`msg_<session>_<idx>` synthetic ids** (`session_manager.rs:1836`) are
    positional, so any history rewrite renumbers messages — fragile for anything
    that wants stable per-message references (e.g. UI anchors, edit provenance).

## Related documentation

- [Context injection and system prompt construction](context-injection-and-system-prompt.md) — the sibling review covering MOIM assembly and hint files; the two overlap on what the model is told about its surroundings.
- [Compaction and context management](compaction-and-context-management.md) — why todos stored in `extension_data` survive a compaction that discards the message history.
- [Shadow git checkpoints design](../../../agent-loop/designs/shadow-git-checkpoints.md) — the BR-43 design that answers gap #2, this review's "single biggest gap".
- [Cross-session memory design](../../../agent-loop/designs/cross-session-memory.md) — the BR-17 design that answers the disjoint-memory gaps #5 and #6.
- [Compaction and memory compared with other agents](../competitive-comparison/compaction-and-memory.md) — how BioRouter's memory stack measures against eight other coding agents.
