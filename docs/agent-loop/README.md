# The agent loop

This folder documents the reasoning loop itself — everything that shapes what the agent
knows, what it is allowed to do, and how it is interrupted or extended while a turn is
running. That spans two audiences deliberately. The user-facing guides here cover the
mechanisms you configure: durable context (memory, skills, workflows), delegation to
subagents, and lifecycle hooks. The developer-facing subfolders hold the designs behind
the loop's guardrails — the command policy engine, sandboxing, checkpoints, session
branching, MCP process pooling and cross-session memory — most of which came out of the
2026-07 agent-loop fix campaign and carry `BR-NN` proposal identifiers.

Come here when you want to change how the agent behaves across a whole session rather
than within one message. Go elsewhere if you are looking for: the **user-facing** rules
on autonomy, admin policy and credentials, which live in
[`docs/security/`](../security/README.md) (this folder holds the *design* behind them, not
the how-to); the **narrative record** of when the campaign work landed and what it proved,
which lives in [`docs/history/agent-loop-campaign/`](../history/agent-loop-campaign/README.md)
— including its [cross-platform arm](../history/agent-loop-campaign/cross-platform/README.md),
which holds the audits and shipped designs behind the loop's Windows and Linux behaviour;
or the **packaged, shareable form** of a configured session, which is
[`docs/workflows/`](../workflows/README.md). Several documents below are plans of record
for work that is only partly built — each states its own status in its header, and the
table repeats it, so check that before treating a design as a description of shipped code.

## Documents in this folder

| Document | What it covers |
|---|---|
| [Context engineering](context-engineering.md) | An index of the features that give the agent durable background knowledge, preferences and workflows — memory, skills, extensions, workflows, config files, environment variables, hooks and delegation — as a routing table pointing at each one's own guide. Current; its original docs-site body was stripped by the 2026-05-07 plain-markdown migration and the routing table replaces it. |
| [The bug reporter](bug-reporting.md) | The design of `platform__report_bug`: how the agent reads a session's own failure record, decides whether it can tell what went wrong, scrubs the report it writes, and files it only behind a proof-backed approval showing the exact body. Covers the two-call shape, the private-session refusal, and the two ways it reaches GitHub. Current. |
| [Subagents](subagents.md) | A guide to subagents — the temporary biorouter instances the main agent spawns to run a task in isolation — covering natural-language invocation, workflow-file configuration, extension and return-mode control, and what subagents are forbidden to do. Current. |
| [Workspace control](workspace-control.md) | The task-oriented guide to running more than one conversation at once: the tab/pane/window layout the agent places work into and the six-pane ceiling, delegating to subagents you can watch, reading and waiting on what is running, reconfiguring another conversation, the caps you will actually meet, and the two distinct ways the CLI reaches the same feature. Current. |
| [Workspace Control tool reference](workspace-control-tools.md) | The precise per-tool contract for the eight tools the Workspace Control capability advertises: arguments and defaults, exact return shapes and refusal strings, which enum arguments are validated and which silently fall back, the daemon-required matrix, and the three honesty gaps found in source — `workspace_close { scope: "tab" }` and a `subagent` tab announcement both report success the renderer may have refused, and the `activate_tab` frame has no production emitter. Current. |
| [Session metadata contract](session-metadata-contract.md) | The canonical identity of a conversation — its `YYYYMMDD_N` id, its `session_type`, its `parent_session_id`, and the two-part rule that makes something a subagent run — plus which surface reads which field, what an export/import round trip does to an id, and the three inferences the identity path deliberately refuses to make. Current; established for [#111](https://github.com/BaranziniLab/biorouter/issues/111). |
| [Conversation writeback freshness](conversation-writeback-freshness.md) | Why every whole-history rewrite of a session goes through a compare-and-swap freshness guard, the exact claim it does and does not make ("a message may only be missing from the verbatim transcript if some compaction was actually shown it"), what each compaction site does when the swap is declined, the three invariants a reviewer must check, and a scoreboard of which deletion paths are closed and which are deliberately left open. Current; the rewrite paths are closed, `edit_message` is locked and bounded with its widest race left to an optional client-view check a client can only satisfy for a session it has re-read ([#59](https://github.com/BaranziniLab/biorouter/issues/59)), and checkpoint restore is open on purpose. See also the preservation marker, which is the other half of the same guarantee. |
| [The compaction preservation marker](compaction-preservation-marker.md) | The per-message marker that carries a message verbatim through every compaction instead of dissolving it into a summary: the exact promise ("never dropped by summarization", not "never dropped"), the single funnel all five compaction sites reach, the two caps that bound the pinned set, oldest-first eviction, and the three signals that make exceeding the bound observable. Current; no producer sets the marker yet. |
| [Tool routing](tool-routing.md) | Which tool the agent should reach for and in what order of preference: the two-tier model, a per-tool "when to use and when not to", the overlap matrix, the three places in source where the same guidance is mirrored, and the always-on `tool_result` log line that makes every dispatched call inspectable. Current, with the tier reading awaiting confirmation and a deprecation proposal awaiting approval. |
| [Turn cancellation and process reaping](turn-cancellation-and-process-reaping.md) | How Stop reaches a running OS process: the cooperative chain from `/agent/cancel` to `killpg`, the extra hop a `code_execution`-nested tool call adds, and the two rules that keep it intact — never abort a future that owes a cancellation downstream, and never assume killing a child kills its descendants. Current; written up after [#72](https://github.com/BaranziniLab/biorouter/issues/72). |
| [The workspace map](workspace-map.md) | The per-turn file map of the working directory: what it contains and what bounds it, the four rules that keep its directory walk off the turn's critical path (no syscall on the async path, a finite budget on the blocking pool, a wait tied to the turn's cancellation token, one walk per root), and the decision that a home directory, a filesystem root and the OS/cloud-sync trees get no map at all. Current; the bounding rules landed after finding M1 of the 2026-09-10 test drive measured every turn blocking in `opendir` under `~/Library/Group Containers`. |

## Subfolders

- [Agent lifecycle hooks](hooks/README.md) — the hook system reference and the shipped
  verify-and-checkpoint Stop hook: the shell commands and LLM judges that run before a
  tool call, around compaction, at session boundaries, and when the agent tries to finish
  a turn.
- [Designs](designs/) — seven subsystem designs from the fix campaign, each stating how
  much of itself has shipped: the [command policy engine](designs/command-policy-engine.md)
  (BR-21, which replaced the evadable `THREAT_PATTERNS` regex table, since deleted; slice 1 live),
  [cross-session memory](designs/cross-session-memory.md) (BR-17; FTS5 chat recall live,
  distillation and digest unbuilt),
  [OS-level tool sandboxing on Linux and Windows](designs/linux-and-windows-sandboxing.md)
  (BR-69; the `ShellSandbox` trait and its macOS and Linux backends shipped, real Windows
  containment did not),
  the [managed policy tier](designs/managed-policy-tier.md) (BR-65; first slice live,
  `verify_trusted()` still a no-op on Windows),
  [session branching](designs/session-branching.md) (BR-45; stable message ids landed, the
  branch tree UX did not), [shadow-git checkpoints and `/rewind`](designs/shadow-git-checkpoints.md)
  (BR-43; capture live behind `BIOROUTER_CHECKPOINTS`, the rewind UI not built), and the
  [shared MCP server pool](designs/shared-mcp-server-pool.md) (BR-54; both slices shipped,
  now the architecture reference for live pooling code).

> **Identifier key.** `BR-NN` identifiers throughout this folder are proposals from the
> campaign's numbering. `BR-1`…`BR-67` are defined in the 67-item master list in
> [the agent-loop improvement proposals](../history/agent-loop-review/improvement-proposals.md);
> `BR-68`, `BR-69` and `BR-70` were added mid-campaign and, like the `GAP-N` per-platform
> findings, are defined in
> [the platform parity audit](../history/agent-loop-campaign/cross-platform/platform-parity-audit.md).

## Related documentation

- [Agent-loop fix campaign](../history/agent-loop-campaign/README.md) — the historical
  record of when the designs here were executed, which waves gated them, and what merged.
- [Agent-loop improvement proposals](../history/agent-loop-review/improvement-proposals.md) —
  the 67-item master list the `BR-NN` identifiers in these designs point back to.
- [Security](../security/README.md) — the user- and admin-facing side of the same
  guardrails: permission modes, managed policy, and secret storage.
- [Biorouter agentic system explorer](../architecture/agentic-system-explorer.md) — the
  code-aligned account of how one request becomes context, tool work, durable state and a
  verified answer, if you want the loop end-to-end before reading a single subsystem.
- [Streaming tool-call UI campaign](../history/streaming-tool-call-ui-2026-07/README.md) — the
  July 2026 campaign that rewrote the loop's streaming decoders and tool dispatch, and wrote
  the tool-routing guidance above.
- [Extensions](../extensions/README.md) — the extensions that supply the tools this folder's
  routing guidance chooses between.
