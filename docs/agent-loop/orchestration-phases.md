# Orchestration phases

> **What this is.** How the main agent runs delegated work in phases (plan, research, implementation, verification and so on) with the `workspace_orchestrate` tool: the plan record, the phases and their kinds, how a subagent is spawned into a phase, the barrier that closes one, the hand-offs that carry results forward, and how each interface shows the plan.
> **Status:** Current.
> **Audience:** end users who ask for a staged run, and developers working on the agent loop or on a surface that draws the plan.

Most delegation needs no plan: the agent calls `subagent`, the child runs in the background, and the parent collects its result. Some work is staged, though. A request such as "research the options, have two agents implement the best one, then verify and audit it" has an order, a point where one stage must finish before the next starts, and results that the later agents need to read. An orchestration plan makes that structure explicit. The agent declares the phases, spawns each subagent into a phase, closes a phase once its subagents are done, and the results of a closed phase reach the subagents of the phases after it.

The plan is optional. The tool's description tells the model to use it when you ask for a multi-phase or workflow-style run, or a plan with stages, and never for a simple delegation.

## When the tool is offered

`workspace_orchestrate` belongs to the workspace extension and is offered exactly where the `subagent` tool is:

- the auto-injected delegation grant (`Agent::ensure_spawn_extension` in `crates/biorouter/src/agents/agent.rs`) lists it beside `subagent`, so a chat that may delegate has it;
- `Agent::list_tools_for` strips it, with the spawn tool, from any chat whose delegation gate is closed, even when Workspace Control is enabled explicitly;
- a subagent session is refused it, like every `workspace_*` tool (`is_workspace_tool_refused_for`), so only the main agent plans;
- a Claude Code or Codex parent reaches it over the coding-agent tool bridge, where it is on `CODING_AGENT_BRIDGE_ALLOWED_WORKSPACE_TOOLS` beside `subagent`. It reads and writes only the calling chat's own plan, a smaller reach than `workspace_send_prompt` and `workspace_close` on the same list, and it is bridged only while delegation is available, like the spawn tool.

The delegation gate itself (Completely Autonomous mode, at least one ordinary extension, not a subagent, not a `gemini` model, not an app that delegates through `consult`) is described in [Workspace control](workspace-control.md#when-delegation-is-allowed).

## The plan record

The plan lives in the parent chat's `extension_data` under the key `orchestration_plan.v1`. Every write goes through `SessionManager::update_extension_state`, the per-key transaction, so it cannot erase another key of the same column. That protects the plan only from writers that are transactions too. The two the model can drive in the same tool batch as a plan action or a spawn, the to-do list (`TodoClient::with_state`) and the goal (`persist_goal`, `clear_persisted_goal`), write through the transactional calls for this reason: as a `get_session` read followed by a whole-blob `update().extension_data()` write, they put back the column as they had read it and erased a plan, or a subagent recorded into one, committed in between. A writer that still has that shape can still do so. A plan written by a newer build that this build cannot parse is preserved, and the write is refused, rather than replaced.

The record holds the parent's decisions only: the plan's title and goal, a revision counter bumped by every write, and for each phase its id, title, goal, kind, status, dependencies, the subagents spawned into it (child session id, role, spawn time), its hand-off and its start and finish times. A child's run status is never copied into it. Each phase's progress is derived when the plan is read, from the chat's subagent roster (`crates/biorouter/src/agents/subagent_roster.rs`), so the plan cannot drift from what the children did.

The bounds, all in `crates/biorouter/src/agents/orchestration_plan.rs`:

| Bound | Value |
|---|---|
| Phases in one plan | 12 |
| Subagents recorded across a plan | 64 (the in-flight spawn cap) |
| Phase id | a slug, `[a-z0-9-]{1,40}` |
| Plan and phase titles | 80 characters |
| Plan and phase goals | 500 characters |
| One hand-off | 4,000 characters |
| Role label | 40 characters, shortened rather than refused |
| Hand-offs a child receives in total | 12,000 characters |

Both counting limits apply to one plan. A plan cannot shed a phase that ran subagents, so once either is reached, further phased work needs a new plan: close every open phase (`complete`, with `status:"skipped"` for work that will not happen), then declare the next plan with `action:"plan"` and `fresh:true`.

## Phases and their kinds

A phase's `kind` is one of `plan`, `research`, `execution`, `implementation`, `verification`, `testing`, `auditing`, `review`, `meeting`, `synthesis` and `other`. It picks the glyph a surface draws and carries no behaviour. Common verb forms (`implement`, `verify`, `test`, `audit`) are accepted, and any word the tool does not recognise is recorded as `other`, because refusing a plan over a glyph would cost a round trip for nothing.

A phase's status is `pending`, `in_progress`, `completed`, `blocked` or `skipped`:

- a new phase is `pending`;
- `action:"start"`, or the first spawn into the phase, moves a pending or blocked phase to `in_progress`;
- `action:"complete"` moves it to `completed`, `blocked` or `skipped`;
- a completed or skipped phase is closed: nothing more can be spawned into it, and it cannot start again. Redoing its work means adding a new phase.

`depends_on` names earlier phases only, which is what rules out cycles. It decides which hand-offs a phase's subagents receive (below). It does not stop a phase starting before its dependencies finish, since overlapping phases are a legitimate pipeline; `action:"start"` says when a dependency is still open.

## The tool's actions

`workspace_orchestrate` takes an `action` and the arguments that action needs. Every successful call answers with the plan as text and as structured content (the `OrchestrationSnapshot` the other surfaces read).

| Action | Arguments | What it does |
|---|---|---|
| `plan` | `phases` (every phase, in order), `title` (required for a new plan), `goal`, optional `fresh` | Declares the plan, or revises it. A phase whose id is kept keeps its subagents, status, hand-off and times; its title, goal, kind and dependencies take the new values. Removing a phase that already ran subagents is refused. With `fresh:true` it replaces a finished plan (every phase completed or skipped) with a new one instead: the new phases start empty even where an id repeats, they receive none of the old plan's hand-offs, the title is kept when none is given, and the old plan's subagents stay in the roster. `fresh:true` is refused while any phase is open. |
| `start` | `phase` | Moves the phase to in progress. |
| `handoff` | `phase`, `summary` | Writes, or replaces, the results the phase carries forward. |
| `complete` | `phase`, optional `status` (`completed`, the default, `blocked` or `skipped`) and `summary` | Closes the phase. This is the barrier. |
| `read` | none | Shows the plan with each phase's progress. |

A refusal is an ordinary tool error the model reads: an unknown phase names the plan's phases, a closed phase names the phases still open, and a bad plan says which phase and which bound.

## Spawning into a phase

The `subagent` tool takes two parameters for this: `phase`, a phase id from the plan, and `role`, a short label ("reviewer", "data loader") that every surface shows beside the child. Before any child session exists, the spawn path asks `orchestration_plan::validate_spawn_phase`, which refuses a spawn when the chat has no plan, when the phase is unknown or closed, or when the plan already records 64 subagents. A refused spawn leaves nothing behind. Once the child's session exists, and before the child runs, `orchestration_plan::record_spawn` appends it to the phase. That record checks the phase again, in the plan's own transaction, because the phase may have changed since the first check: a `complete` may have closed it, a revision may have dropped it, or a parallel spawn may have taken the plan's last place. A record refused for any of these reasons deletes the child's new session and refuses the spawn, so no subagent runs outside the plan its parent named.

## The barrier

`action:"complete"` is refused while any subagent in the phase is still queued, running or being cancelled. The refusal lists those subagents and gives the `workspace_watch` call that waits for all of them (`mode:"all"`). The tool description tells the model to complete a phase only after that watch, so a phase is closed over finished work rather than over work in flight. A skip is held by the same rule; a subagent that should not finish is stopped with `workspace_close` first.

Each child's state is read in this order: a background handle this process is running (with no result yet, or a pending Stop-and-Send continuation) counts as active whatever else says; then the roster row; then a finished handle's own result; then the child's durable outcome record. A child none of them knows is unknown, and the barrier does not hold on it, because nothing in this process is running it. The states are read before the write, and the write is refused if a subagent was recorded into the phase in between, so a spawn racing the completion cannot be closed over unchecked.

A spawn into the phase that is still starting is held by the barrier too. Between its first phase check and the moment its child is visible to the barrier (a registered background handle, or on the blocking path the end of the child's run), the spawn holds a process-local guard, `orchestration_plan::begin_phase_spawn`. `complete` reads the guard before it reads the children and again inside its write, and refuses while one is held or when one was taken in between. Without it, a `complete` in the same tool batch as a spawn could read the phase before the starting child was recorded, or before its handle existed, and close the phase over it.

## Hand-offs: how later phases meet earlier results

Subagents cannot read each other: every `workspace_*` tool is refused to a subagent, and that refusal is deliberate. Results move between phases through the parent instead, as hand-offs. These hand-offs are the "meeting" between phases.

- The parent writes a hand-off with `action:"handoff"`, or passes `summary` to `action:"complete"`.
- When the parent completes a phase with no hand-off written, one is composed from the phase's subagents: one line per subagent with its role, status and the first line of its result.
- A subagent spawned into a phase receives a "Results from earlier phases" block in its instructions, placed after its task. It holds the hand-offs of the phases the phase names in `depends_on`, whatever their status, or, when it names none, of every earlier phase that has completed. The block is capped at 12,000 characters.

A hand-off reaches only subagents spawned after it was written. A `meeting` phase is the explicit form: its subagents start from the earlier phases' hand-offs and reconcile them.

### A subagent more private than its parent

A spawn never changes a chat's privacy tier, but a subagent that reaches a private model or data source during its run becomes private while its parent stays public. Nothing that subagent wrote may reach the parent through the plan. When `orchestration_plan::phase_agent_states` reads a phase's subagents it compares each one's tier with the parent's, as both session rows record them at that moment, and marks a more private subagent as withheld:

- its result line is dropped before anything is composed, so the automatic hand-off shows "(result withheld)" in its place;
- it is labelled by the role its parent gave it, or by its session id, never by its session name, which a person may have renamed in its own tab;
- the hand-off says how many results were withheld, right after its heading, where the length cap cannot cut it off, and the reply to `action:"complete"` says so too.

A subagent whose tier cannot be read is withheld when it has a result to show. Its run status still counts toward the barrier and the phase's progress, because the parent spawned it and already knows it exists. Everything else that shows plan text (the per-turn summary, the tool's replies, the desktop and terminal views, and the hand-offs later subagents receive) is rendered from the stored hand-off, so it carries only what the composition kept. Once the parent is itself private, nothing is withheld.

## How each interface shows the plan

Every surface reads one function, `subagent_roster::session_subagents`, whose result carries the subagent rows and the plan snapshot, so no two surfaces can disagree about a phase's progress.

- **The model** sees a compact plan at the top of every turn, one line per phase with its status and progress, added by the workspace extension's per-turn context only when a plan exists. `ExtensionManager::collect_moim` asks each platform extension with the entry the chat holds (`McpClientTrait::get_moim_for`), and the workspace extension shows each part only where that entry grants its tool: the plan where it grants `workspace_orchestrate`, the preview panel where it grants `workspace_read_panel`. The auto-injected delegation grant holds the first and not the second, so a delegating chat sees its plan and the renderer is never asked for its panel.
- **The desktop app and the browser (`biorouter serve`)** show the phases in the chat summary rail and in its header popover, each with its subagents and a progress line, beside the chat's other subagents. See [Chat summary rail](../desktop-ui/chat-summary-rail.md).
- **The daemon** serves the same data at `GET /sessions/{session_id}/subagents`. The parent is gated with `session_reach`, each child is filtered on its own privacy tier, and the plan is recomputed from the children that survive, so a hidden child is neither counted nor listed.
- **The CLI** prints the plan above the subagent list with `biorouter session subagents <session_id>` (`--format json`, `--watch` to redraw every 2 seconds). With no daemon it reads the local store, and a subagent that was running reads as interrupted or unknown.
- **The terminal UI** shows the phases in its subagent pane (`Ctrl-B` to show or hide it, `Ctrl-O` to move focus into it).

A phase's progress counts its subagents as done (completed), running (queued, running or cancelling), blocked, failed (failed, incomplete or interrupted) and quiet. Quiet means a subagent is active but has recorded no activity for 3 minutes. It is a heuristic and is labelled as such, never as stalled.

## Related documentation

- [Subagents](subagents.md): the spawn tool, its parameters, and how a subagent runs in the background.
- [Workspace control](workspace-control.md): delegating, watching and stopping subagents, and the delegation gate.
- [Workspace Control tool reference](workspace-control-tools.md): the exact arguments and refusals of `workspace_orchestrate` and the other workspace tools.
- [Chat summary rail](../desktop-ui/chat-summary-rail.md): where the desktop app draws a chat's subagents and phases.
- [Session metadata contract](session-metadata-contract.md): what makes a session a subagent of a parent.
