# Tool routing — which tool for which job

> **What this is.** The canonical reference for tool selection: which tool the agent should
> reach for, in what order of preference, where the overlaps are, and where every dispatched
> tool call's outcome can be inspected.
> **Status:** Current, with two items open — the tier model in the next section is an
> interpretation awaiting confirmation, and the deprecation proposal near the end has been
> approved by nobody and removed nothing.
> **Audience:** developers working on the agent loop, extension authors deciding how to describe
> a tool, and agents choosing a tool at runtime.

BioRouter exposes several tools that can each list a directory, read a file, or run a command.
Without explicit routing guidance the model picks the wrong one — writing a JavaScript script to
`ls` a directory or copy a file, or calling a tool-discovery search to answer a web question. This
page states the preference order once, and names the three places in the source where the same
guidance is mirrored so they can be kept in sync.

The user's directive: *"when the situations are somewhat more straightforward and
easy, we should almost always rely on the most basic fundamental tools inside of
BioRouter — the tools without any extensions. Only when there are more complicated
tasks should we think about using the extensions."*

## Interpretation: a tier model, pending confirmation

There is an important ground-truth wrinkle behind the directive. **BioRouter has no
truly extension-free `shell`, `edit`, or `find` tool.** The tools the user thinks of
as "fundamental" — `shell` and `text_editor` — are provided by the **`developer`
extension** (`crates/biorouter-mcp/src/lib.rs`, `BUILTIN_EXTENSIONS["developer"]`),
which is a real, loadable, disable-able extension that merely happens to be enabled
by default in essentially every session. The only always-present, genuinely
extension-free tools are the gated `platform__*` tools (`ingest_conversation`,
`manage_schedule`, `read_session_blob`) and `workflow__final_output`. Delegation is
no longer in that list at all: since BR-71 the spawn tool is advertised by the
`workspace` extension as `workspace__subagent` (auto-injected when delegation is
enabled), and it still paradoxically requires at least one extension to be loaded.
None of those do file or shell work.

So "tools without any extensions" cannot be read literally. The faithful reading of
the directive is a **tier model** by simplicity, not by extension-vs-not:

- **Tier 1 — primitives.** The simplest tool that directly does the job. For file
  and system work this is the `developer` extension: `shell` and `text_editor`. These
  are the default, first-choice tools for anything "straightforward and easy."
- **Tier 2 — specialized extensions.** Reach here only when a task genuinely needs a
  capability a primitive lacks: real computation / control flow / multi-call chaining
  (`code_execution`), figures (`autovisualiser`), a knowledge base (`knowledge` /
  `bokf`), browser automation (`playwright`), data queries, app building
  (`agent_drafter`), and so on.

**This tier reading is an interpretation — please confirm or correct it.** If instead
you want the literal set (only `platform__*` and friends treated as "fundamental"),
the routing guidance would look very different, because those tools cannot list or
edit files at all.

## Tier table

| Tier | Tools | Reach for it when… |
|------|-------|--------------------|
| **1 — primitives** | `developer/shell`, `developer/text_editor` | Listing, reading, writing, editing, copying, moving, deleting, or finding files; running one-off commands; anything straightforward. **Default.** |
| **1 — always-on platform** | `platform__ingest_conversation`, `platform__manage_schedule`, `platform__read_session_blob` | Saving a conversation to a knowledge base (KB); scheduling a workflow; re-reading a large externalized tool output. |
| **1 — workspace control** | `workspace_list`, `workspace_open`, `workspace_read_conversation`, `workspace_send_prompt`, `workspace_set_tools`, `workspace_close`, `workspace_watch`, `subagent` | Operating the conversations themselves — inspecting, opening, steering, reconfiguring or closing another chat; waiting on background work; delegating a bounded sub-task. The user enables the `workspace` extension to get the whole set. Enabling delegation alone auto-injects only `subagent` plus child-scoped read/watch/close, so broad listing, steering, reconfiguration, and session creation still require Workspace Control. See the routing table below. |
| **2 — computation / chaining** | `code_execution` (`execute_code`, `search_modules`, `read_module`) | Several **dependent** tool calls whose outputs feed each other in one round-trip, or real loops/aggregation/conditionals over their results. |
| **2 — specialized domains** | `autovisualiser/*`, `knowledge`/`bokf_*`, `computercontroller/*`, `playwright/*`, `agent_drafter` (`files_server`, `compute_server`, `appcontrol`), data-query extensions | The task is squarely in that extension's domain (a figure, a knowledge base, GUI/browser automation, an app sandbox). |

## Per-tool "when to use / when not to"

### `developer/shell`
- **Use for:** running commands; `ls`/`cp`/`mv`/`rm`/`mkdir`; finding files or text
  with `rg` (never `find` or `ls -r`); chaining a couple of commands with `&&`;
  long-lived processes via `background=true` + `shell_wait`/`shell_output`/`shell_kill`.
- **Don't:** use it to dump a whole file to read it (`cat`/`head`) — use
  `text_editor` view; produce huge output (pipe to a file); busy-loop with `sleep`
  (use `shell_wait`).

### `developer/text_editor`
- **Use for:** reading a file (`view`), creating/overwriting (`write`, full content),
  editing (`str_replace`, or a multi-file unified `diff` in one call; `insert`).
- **Don't:** read/write files via shell `cat`/`sed`/`echo >`, or via a code-execution
  script, when this tool does it directly.

### `code_execution/execute_code`
- **Use for:** two or more **dependent** MCP (Model Context Protocol) tool calls in one round-trip; running
  computation, loops, conditionals, or aggregation over tool outputs.
- **Don't:** list a directory, read/write a single file, or copy/move/delete — call
  `developer/shell` or `developer/text_editor` directly. Don't wrap a single tool call
  in a script.

### `code_execution/search_modules`, `read_module`
- **Use for:** discovering which installed MCP tool/module to `import` inside an
  `execute_code` script, and its signature.
- **Don't:** treat `search_modules` as a web/knowledge/general search. It searches the
  **local tool catalog** only and answers no questions. For a factual or web-research
  question, use a web tool or answer directly.

### Native Biorouter Copilot and Web & Documents
- Use `computercontroller/list_apps` then `get_app_state` before native element actions.
  `screen_capture` is the sole built-in display/window capture tool. Follow the current schemas.
- Obtain the host's per-chat task grant once, continue authorized work, and stop on denial or
  revocation. Never use shell or another automation route to bypass a Biorouter Copilot block.
- Keep desktop calls sequential and refresh observations after handoff, stale references, or
  uncertain completion. Never replay a timed-out mutation blindly.
- Use `webdocuments/web_scrape` for a known URL and `webdocuments` document/cache tools for
  those formats. They do not require the desktop helper. Ordinary files/code use Developer.
- Private and public chats retain separate observations and grants. A shared physical desktop
  can expose content left visible; acknowledge a private-to-public handoff before capture.

### `files_server` / `compute_server` (Agent Drafter app sandboxes)
- **Use for:** file and shell/Python work **scoped to a built app's sandbox**.
- **Don't:** use them as general file/shell tools for the user's own workspace — that
  is `developer`'s job.

### `workspace_*` / `subagent` (Workspace Control) vs. `chatrecall` vs. Memory

The workspace extension operates the **live** workspace — the conversations
themselves. It is routinely confused with three neighbours that all touch "other
conversations" in some sense, so route by *what is being asked for*, not by the
word "conversation" appearing in the question:

| The user wants… | Route to | Not to |
|-----------------|----------|--------|
| The **content** of a past chat ("what did we conclude about the volcano plot last week?") | `chatrecall` (search by query, or load a session by id) | `workspace_read_conversation` — it reads a session you already identified, it does not search by content |
| A chat the user names by **exact id** ("what happened in 20260823_2?") — they can copy one from any chat row's right-click menu | `chatrecall`'s **load** mode with that `session_id`, then `workspace_read_conversation` if more than a head/tail is needed | a `chatrecall` *query* on the id or the chat's title. An exact handle was supplied so nothing has to be guessed; a title search can match a different chat and look like it worked |
| **Live control** of another chat, or a **structured read** of one ("what is that other conversation doing right now?") | `workspace_list`, then `workspace_read_conversation` with `view:"tool_calls"` | a `chatrecall` load, which returns only the first/last few messages |
| To **change another chat's setup** ("give that other conversation the single-cell skill") | `workspace_set_tools` with `add_skills` / `add_extensions` / `set_knowledge_bases` — do it, session-scoped | telling the user to open Settings, which is machine-wide and is not the ask |
| To **delegate** a bounded sub-task ("delegate checking the test suite to a subagent I can watch", "spin up three sub-agents") | `subagent` — the one spawn tool, advertised by the workspace extension, and the only thing that creates a `sub_agent` session with a parent | `workspace_open { new: { prompt } }`, which creates a conversation **the user** owns with no parent, and which now refuses `new.kind:"sub_agent"` outright ([#111](https://github.com/BaranziniLab/biorouter/issues/111)) |
| To open **another chat for the user** ("start a separate chat for the figure work") | `workspace_open { new: { kind: "user", … } }` | `subagent`, which makes the new conversation the agent's delegate rather than the user's own |
| To **wait on background work** ("tell me as soon as one of those three background jobs is done") | `workspace_watch` on the session ids | a `workspace_read_conversation` poll loop — the failure mode the `subagent_status` → `workspace_watch` migration exists to remove |
| To **remember a durable fact** ("remember that I prefer uv over pip") | Memory (`remember_memory`) | any workspace tool; workspace state is per-conversation and transient |
| To **fold a conversation into a knowledge base** | `platform__ingest_conversation` | `workspace_read_conversation` followed by a hand-written KB write |
| To **re-read a large externalized tool output** | `platform__read_session_blob` | re-running the tool that produced it |

Names above are the tools' own names, as the extension registers them; on the wire
the agent sees them prefixed with the extension — `workspace__workspace_list`,
`workspace__subagent`, and so on.

- **Don't:** use `workspace_read_conversation` as a search engine, or poll it in a
  loop when `workspace_watch` will park until something finishes.
- **Do:** treat another conversation's content as sensitive — read the narrowest
  view (`summary` before `transcript`, `tool_calls` when the question is "what did
  it *do*") and only what the task needs.

#### The two sides of this split are not equally guarded

The `chatrecall` side is enforced. [Privacy tiers](../security/privacy-tiers.md) put **Gate D**
inside it, in both modes: SEARCH carries the caller's tier into the SQL, so a chat running on a
public model never matches a row belonging to a private conversation; LOAD checks the named
session's tier *before* the header string is built, so neither the session's name nor its working
directory escapes with the refusal. A refusal is returned as text the model reads, not as an error,
and the wording lives in one place (`crates/biorouter/src/privacy/refusal.rs`) so it cannot drift.
The refusal carries **no** content from the session it refused.

The `workspace_*` side is **not** enforced today, and that asymmetry is the reason this subsection
exists. `workspace_read_conversation` loads any session it is given by id and checks only whether
that session is `Hidden` — it does not consult `privacy_tier`. The predicate that would close it
exists (`crates/biorouter/src/privacy/visibility.rs`, design §7's `may_read`) and no handler calls
it. So a model can reach through the workspace tools for a transcript that chat recall would have
refused it.

Two consequences for routing, until that lands:

- **Do not treat "chat recall refused it" as "that content is unreachable."** It means chat recall
  refused it. Reaching for the same content through `workspace_read_conversation` is routing around
  a privacy control, and the "read the narrowest view" rule above is what stands in for the missing
  gate.
- **If you are adding a tool that reads another conversation**, it inherits this gap by default.
  Call `privacy::visibility::may_read` with the caller's capability and the target's stored
  classification; do not re-derive the rule.

One write is covered, and it is worth knowing which: `workspace_set_tools { provider, model }` calls
the same `Agent::update_provider` the model picker does, so **Gate A** applies to it and steering
another chat onto a public model cannot launder a private one. Design §7's write row is `may_write`, and it
is `VIS` — the same predicate as READ. The **lineage conditions** that used to sit on
`workspace_send_prompt` and on the rest of `workspace_set_tools` are **retired**: an agent may
inject into any conversation it can see, and only the privacy tier refuses.

The same guidance is mirrored in the extension's own `INSTRUCTIONS` block
(`crates/biorouter/src/agents/workspace_extension.rs`), which a unit test holds to
≤2,800 characters and to naming only tools `get_tools()` actually registers.

## Overlap matrix

Rows are capabilities; cells mark tools that can do it. **Bold** = the tool to prefer.

| Capability | developer | code_execution | webdocuments | files/compute_server |
|------------|-----------|----------------|--------------------|----------------------|
| List a directory | **shell `ls`** | execute_code | — | list_dir (sandbox) |
| Read a file | **text_editor view** | execute_code | — | read_text_file (sandbox) |
| Write a file | **text_editor write** | execute_code | — | write_text_file (sandbox) |
| Copy/move/delete | **shell** | execute_code | — | — |
| Find files/text | **shell `rg`** | execute_code | — | — |
| Run one command | **shell** | execute_code | — | compute_server/shell (sandbox) |
| Chain N dependent calls + logic | (serial calls) | **execute_code** | — | — |
| Fetch a URL | shell `curl` | execute_code fetch | **web_scrape** | — |
| Run Python | shell `python3` | execute_code | — | **compute_server/python** (sandbox) |

The rule the matrix encodes: **the leftmost bold cell wins for a simple task**;
`execute_code` wins only in the "chain N dependent calls + logic" row.

## Where the guidance now lives (so it stays in sync)

- **System prompt** — `crates/biorouter/src/prompts/system.md`, `# Tool Routing`
  section (renders in every mode, including code-execution mode where per-extension
  instructions are hidden). A one-line version is in
  `crates/biorouter/src/prompts/system_small_local.md`.
- **code_execution extension** — server `instructions` and the `execute_code` /
  `search_modules` tool descriptions in
  `crates/biorouter/src/agents/code_execution_extension.rs`.
- **developer extension** — base `instructions` and the `shell` tool description in
  `crates/biorouter-mcp/src/developer/rmcp_developer.rs`.
- **workspace extension** — the `INSTRUCTIONS` block in
  `crates/biorouter/src/agents/workspace_extension.rs`, whose closing `Routing:`
  sentences are the compressed form of the table above (chatrecall for content,
  Memory for durable facts, `ingest_conversation` for fold-into-KB,
  `read_session_blob` for externalized payloads).

---

## Native desktop replacement

The previous script-driven desktop tools and Developer capture implementation are removed.
There are no compatibility aliases. Current model guidance, nested imports and bridge policy
must use the native Biorouter Copilot roster and independent Web & Documents identity. Existing
script workflows need an explicit rewrite; they do not authorize the new desktop capability.

## Tool-result logging (observability of every tool call)

There is now **one always-on place where every dispatched tool call's outcome is
inspectable**, regardless of extension, error surface, or interface (GUI/CLI).
`Agent::dispatch_tool_call` (`crates/biorouter/src/agents/agent.rs`) emits a single
structured `tracing` line at **`info`** level, keyed **`target: "tool_result"`**,
immediately after the tool future resolves — the universal choke point every tool
call passes through:

- `tool` — the tool name (e.g. `code_execution__execute_code`, `developer__text_editor`)
- `id` — the request id
- `ok` — `true` / `false`
- `dur_ms` — execution duration (excludes time parked on the concurrency semaphore)
- `error` — present only on failure; the error message text

It captures **both** error surfaces: a hard `Err(ErrorData)` (e.g. the developer
`text_editor` jail's `INVALID_PARAMS "… is outside the working directory"`) **and**
an `Ok(CallToolResult)` flagged `is_error: Some(true)` (the MCP "tool ran and reported
a failure" path, e.g. a `code_execution` script whose inner tool failed). Kept cheap:
the error string is only materialised on the failure path; the success path logs no
payload.

Filter the logs with `RUST_LOG=tool_result=info` (or grep `target="tool_result"`)
to get a clean, one-line-per-call ledger of what the agent did and what failed. This
complements the pre-existing `TOOL_EXEC_START`/`TOOL_EXEC_END` **`debug`** markers
(timing only, no ok/error) and the raw `rmcp::service` `WARN response error …` lines
(per-server, no tool-name/ok context).

## Related documentation

- [Session metadata contract](session-metadata-contract.md) — the identity the workspace tools and Chat Recall both resolve ids against, and the rule that separates a delegation from a conversation the user owns.
- [The agent loop](README.md) — the loop that dispatches every tool call routed by this page.
- [Extensions and skills](../extensions/extensions-and-skills-guide.md) — how the extensions providing these tools are installed, enabled and described.
- [Built-in extensions](../extensions/built-in/README.md) — the reference page for each shipped extension named in the tier table.
- [Privacy tiers](../security/privacy-tiers.md) — Gate D inside the `chatrecall` route above, Gate C at the dispatch choke point every tool on this page passes through, and the §7 matrix the workspace tools do not yet call.
- [Streaming tool-call UI campaign](../history/streaming-tool-call-ui-2026-07/README.md) — the July 2026 campaign that wrote this guidance and the tool-result logging described above.
- [Tool-errors audit](../history/streaming-tool-call-ui-2026-07/tool-errors-audit.md) — the log sweep that motivated the always-on `tool_result` line.
