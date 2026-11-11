# Orchestration phases

> **What this is.** How the main agent runs delegated work in phases (plan, research, implementation, verification and so on) with the `workspace_orchestrate` tool: the plan record, the phases and their kinds, how a subagent is spawned into a phase, the barrier that closes one, the decision gate that holds a phase, and every phase after it, until a decision model (or, without one, the chat's own model) is confident it met its goal or the person decides, the monitors a phase gives its subagents, plans a workflow declares, the hand-offs that carry results forward, and how each interface shows the plan.
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

The record holds the parent's decisions only: the plan's title and goal, a revision counter bumped by every write, and for each phase its id, title, goal, kind, status, dependencies, the subagents spawned into it (child session id, role, spawn time), its hand-off, its start and finish times, and its decision gate with the record of the gate's evaluations. A child's run status is never copied into it. Each phase's progress is derived when the plan is read, from the chat's subagent roster (`crates/biorouter/src/agents/subagent_roster.rs`), so the plan cannot drift from what the children did.

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
| Monitors on one phase | 4 |
| Hand-offs a child receives in total | 12,000 characters |

Both counting limits apply to one plan. A plan cannot shed a phase that ran subagents, so once either is reached, further phased work needs a new plan: close every open phase, held ones included (`complete`, with `status:"skipped"` for ungated work that will not happen; a gated phase closes only through its gate or the person), then declare the next plan with `action:"plan"` and `fresh:true`. Both refusals name the gated open phases apart from the others: "Phase build has a gate, so it closes only through its gate or the person, who can pass or skip it in the app or with `biorouter session gates`."

## Phases and their kinds

A phase's `kind` is one of `plan`, `research`, `execution`, `implementation`, `verification`, `testing`, `auditing`, `review`, `meeting`, `synthesis` and `other`. It picks the glyph a surface draws and carries no behaviour. Common verb forms (`implement`, `verify`, `test`, `audit`) are accepted, and any word the tool does not recognise is recorded as `other`, because refusing a plan over a glyph would cost a round trip for nothing.

A phase's status is `pending`, `in_progress`, `completed`, `blocked` or `skipped`:

- a new phase is `pending`;
- `action:"start"`, or the first spawn into the phase, moves a pending or blocked phase to `in_progress`, unless an earlier gate holds it ([Decision gates](#decision-gates));
- `action:"complete"` moves it to `completed`, `blocked` or `skipped`;
- a completed or skipped phase is closed: nothing more can be spawned into it, and it cannot start again. Redoing its work means adding a new phase.

`depends_on` names earlier phases only, which is what rules out cycles. It decides which hand-offs a phase's subagents receive (below). It does not stop a phase starting before its dependencies finish, since overlapping phases are a legitimate pipeline; `action:"start"` says when a dependency is still open.

## The tool's actions

`workspace_orchestrate` takes an `action` and the arguments that action needs. Every successful call answers with the plan as text and as structured content (the `OrchestrationSnapshot` the other surfaces read).

| Action | Arguments | What it does |
|---|---|---|
| `plan` | `phases` (every phase, in order, each with an optional `gate` and `monitors`), `title` (required for a new plan), `goal`, optional `fresh` | Declares the plan, or revises it. A phase whose id is kept keeps its subagents, status, hand-off and times; its title, goal, kind and dependencies take the new values (an open gated phase keeps its title, goal and kind), and a gate or monitors left out of the revision stay as they were. Removing a phase that already ran subagents is refused. With `fresh:true` it replaces a finished plan (every phase completed or skipped) with a new one instead: the new phases start empty even where an id repeats, they receive none of the old plan's hand-offs, the title is kept when none is given, and the old plan's subagents stay in the roster. `fresh:true` is refused while any phase is open. |
| `start` | `phase` | Moves the phase to in progress. |
| `handoff` | `phase`, `summary` | Writes, or replaces, the results the phase carries forward. |
| `complete` | `phase`, optional `status` (`completed`, the default, `blocked` or `skipped`) and `summary` | Closes the phase. This is the barrier. On a gated phase, `completed` first asks the gate, and `skipped` is refused. |
| `read` | none | Shows the plan with each phase's progress. |

A refusal is an ordinary tool error the model reads: an unknown phase names the plan's phases, a closed phase names the phases still open, and a bad plan says which phase and which bound.

## Spawning into a phase

The `subagent` tool takes two parameters for this: `phase`, a phase id from the plan, and `role`, a short label ("reviewer", "data loader") that every surface shows beside the child. Before any child session exists, the spawn path asks `orchestration_plan::validate_spawn_phase`, which refuses a spawn when the chat has no plan, when the phase is unknown or closed, when an earlier gate holds it, or when the plan already records 64 subagents. A refused spawn leaves nothing behind. The refusal tells the agent which kind of mistake it was: a phase the plan does not name, or a chat with no plan, is the call's fault (`invalid_args`, fix the call); a phase that is held by a gate, closed or full is a precondition (`tool_failure`, "Read the message and address its cause"), because the arguments were right and the same spawn works once the gate passes or the phase is open. Once the child's session exists, and before the child runs, `orchestration_plan::record_spawn` appends it to the phase. That record checks the phase again, in the plan's own transaction, because the phase may have changed since the first check: a `complete` may have closed it, a revision may have dropped it, or a parallel spawn may have taken the plan's last place. A record refused for any of these reasons deletes the child's new session and refuses the spawn, so no subagent runs outside the plan its parent named.

## The barrier

`action:"complete"` is refused while any subagent in the phase is still queued, running or being cancelled. The refusal lists those subagents and gives the `workspace_watch` call that waits for all of them (`mode:"all"`). The tool description tells the model to complete a phase only after that watch, so a phase is closed over finished work rather than over work in flight. A skip is held by the same rule; a subagent that should not finish is stopped with `workspace_close` first.

Each child's state is read in this order: a background handle this process is running (with no result yet, or a pending Stop-and-Send continuation) counts as active whatever else says; then the roster row; then a finished handle's own result; then the child's durable outcome record. A child none of them knows is unknown, and the barrier does not hold on it, because nothing in this process is running it. The states are read before the write, and the write is refused if a subagent was recorded into the phase in between, so a spawn racing the completion cannot be closed over unchecked.

A spawn into the phase that is still starting is held by the barrier too. Between its first phase check and the moment its child is visible to the barrier (a registered background handle, or on the blocking path the end of the child's run), the spawn holds a process-local guard, `orchestration_plan::begin_phase_spawn`. `complete` reads the guard before it reads the children and again inside its write, and refuses while one is held or when one was taken in between. Without it, a `complete` in the same tool batch as a spawn could read the phase before the starting child was recorded, or before its handle existed, and close the phase over it.

## Decision gates

A phase may carry a decision gate: a decision model reads what the phase produced, and the phase completes only when the model is confident, question by question, that the phase met its goal. A gate never passes on an error. What decision models are, and which one a gate uses, is in [Decision models](decision-models.md). A gate does not need a decision model: one that names no model is judged by the chat's own language model when the chat has none ([A gate needs only a language model](#a-gate-needs-only-a-language-model)).

**A gate binds the plan.** Once a phase has a gate, whoever set it, the agent cannot change, remove or skip it, and every phase listed after the gated phase waits: it cannot start, take a subagent or complete as `completed` until the gate passes or the person passes or skips the gated phase. The hold goes by position in the plan, not by `depends_on`, and it ends when the gated phase is completed or skipped. Work outside the plan is not held. The tool's description says so to the model in one paragraph, "A gate binds you", so the agent plans around it rather than learning it from a refusal.

### Who sets a gate

| Owner | How | Threshold |
|---|---|---|
| The agent | A `gate` on a phase in `workspace_orchestrate action:"plan"` | At least 0.50: below that a gate passes a phase the decision model thinks more likely unfinished than done |
| The person | The gate dialog in the app, `biorouter session gates <id> set`, or `PUT /sessions/{session_id}/phase-gates/{phase_id}` | Any, from 0 to 1 |
| A workflow | A `gate` in the workflow's `orchestration_plan` ([Plans from a workflow](#plans-from-a-workflow)) | Any, from 0 to 1 |

Every stored gate binds the agent the same way, whoever owns it. The plan text the model reads, and every surface, says who set it ("set by the person", "set by the workflow").

### Declaring one

Each phase in `action:"plan"` takes an optional `gate`:

```json
{"id": "build", "title": "Build the parser", "kind": "implementation",
 "goal": "A parser with passing tests",
 "gate": {"questions": {"tests_pass": "Do the results show every new test passing?",
                        "scope_met": "Did the phase deliver every item in its goal?"},
          "threshold": 0.8, "max_attempts": 3}}
```

| Field | Value |
|---|---|
| `questions` | 1 to 8 yes/no questions, name to question |
| `threshold` | the P(yes) every question must reach, 0 to 1, default 0.8; at least 0.50 for a gate the agent declares |
| `max_attempts` | evaluations that may fail before the phase is blocked, 1 to 5, default 3 |
| `model` | optional, as `provider/model` or by the bare name or alias `decide` takes (`tev1:0.8b`, `joey-1`) when it names exactly one model the chat may use; stored as `provider/model`. A bare name that names none, or several, refuses the plan: "Phase "verify"'s gate model: … Nothing was changed." Left out, the chat's default decision model, the configured one, or the first one the chat may use, and with none the chat's own model. A workflow file's gates still need `provider/model` |
| `allow_uncalibrated` | accept probabilities a model wrote rather than its own distribution, default false |

**The model is checked when the gate is declared.** A gate or a phase monitor the plan adds is checked the way a saved monitor is ([What a save checks](decision-models.md#what-a-save-checks)), with nothing of the chat sent. Only a gate that names a model can be refused: "Phase "build"'s gate cannot be answered in this chat: … Declare it without that model to let this chat's own model judge it, or ask the person to set a decision model this chat may use. Nothing was changed." A refused gate refuses the whole call. A gate that names no model is accepted in every chat; when the chat has no decision model it may use, the reply says "Phase "build"'s gate is judged by this chat's own model when the phase completes, because this chat has no decision model it may use now." A check that cannot be made just then (a timeout, or a model server that is unreachable, busy or failing) is accepted with a note, "Phase "build"'s gate model could not be checked just now (…); the gate is asked when the phase completes." A stored gate repeated unchanged is not checked again, whoever set it; with `fresh:true` every gate in the new plan is new. A plan that changes while its additions are checked is refused, and nothing is written. Whether a model's probabilities are calibrated is judged when it answers.

**What the agent may still do with a gated plan:**

- declare a gate on a phase that has none, with a threshold of at least 0.50;
- repeat a stored gate unchanged, or leave it out of a revision, which keeps it and its record;
- change `depends_on`, and add phases anywhere;
- mark a gated phase `blocked`: the gate is not asked, no attempt is used, and the phases after it stay held;
- skip or block a held phase that has no gate of its own.

**What it may not do**, each refused with a sentence that says why and that nothing was changed:

- change or remove a gate: "Phase "build" already has a gate, and a revision cannot change or remove a gate. Leave `gate` out of this phase to keep it, or ask the person to change it in the app or with `biorouter session gates`. Nothing was changed." (with ", set by the person" or ", set by the workflow" after "gate" when it is theirs);
- drop an open gated phase from the plan;
- change an open gated phase's title, goal or kind, which its gate reads (a goal or kind left out is kept);
- move a phase from after an open gate to before it: "Phase "verify" waits for phase "build"'s gate and cannot be moved ahead of it. Nothing was changed.";
- skip a gated phase;
- start, spawn into, or complete as `completed` a held phase: "Phase "verify" waits for phase "build"'s gate: it cannot start, take a subagent or complete until that gate passes or the person passes or skips "build". Work on "build" first. Phases you can work on now: …".

A gated phase that passed its gate, or that the person closed, holds nothing, so the rules for open gated phases no longer apply to it.

### A gate needs only a language model

The app needs only one language model, and a gated plan works the same way with or without decision models. A gate that names no model asks the decision model the chat would use. When the chat has none it may use (none is set up, or none it may use at its privacy tier), or that model cannot be reached (a connection or certificate failure, a timeout, or a server error), the chat's own language model judges the gate instead (`orchestration_plan::judge_gate`). It reads the same text a decision model would ([What the gate reads](#what-the-gate-reads)) in one short completion that is not streamed, and answers each question yes or no with its probability of yes, as one JSON object. The answer is read strictly. Anything other than one JSON object answering every question, yes or no with a probability from 0 to 1 that agrees with it, is an evaluation error, and an error never passes. A verdict is then treated as a decision model's: the same threshold, the same attempts, and the same pass, skip and retry for the person. Its probabilities are the model's own estimate, so the gate's setting for uncalibrated probabilities does not apply to it.

The chat's own model is the one the chat is bound to, at the chat's own tier, so nothing crosses a privacy boundary: it is the model that reads the phase's results anyway.

The gate's record keeps who gave its last verdict and why, in one plain sentence, and every surface shows it. The agent's reply ends with the sentence, the gate dialog shows it under Judged by, and `biorouter session gates` prints it under the gate:

- "This chat has no decision model it may use, so gpt-5.5, the chat's model, judged the gate."
- "joey-1 on Baseten could not be reached because its server's certificate did not match, so gpt-5.5, the chat's model, judged the gate."

A gate that names a decision model asks only that model. One the chat may not use is refused when the gate is declared or set, and one that cannot be reached is an evaluation error that waits for the person, as before. A refusal, an answer that could not be read and a rate limit are errors whoever was asked, never a reason to ask the chat's model. Monitors never fall back to the chat's model.

### What the gate reads

One text, at most 6,000 characters and fitted to the gate model's window: the phase's title, kind and goal, its hand-off as it would be stored on completion (the summary given, the one written earlier, or the one composed from the subagents' result lines), and, when the parent wrote the hand-off, each subagent's result line. These are the same screened lines the automatic hand-off uses, so a subagent more private than its parent contributes its role and status and never its text. The decision goes through the decision service as the parent chat, at the parent's tier, with the privacy master switch as this tool call sampled it, so a private chat's phase never reaches a public decision model. When the chat's own model judges the gate, it reads the same text.

**Fitted to the gate model's window.** The text is narrowed from its end, by the same count the decision service's size check uses (its margin, its 15% and one token for every digit), until it fits the model's window beside the questions, so the phase, its goal and the start of its hand-off are kept first. It is never narrowed below 400 characters (`GATE_STATE_MIN_CHARS` in `orchestration_plan.rs`), and a model too small even for that is refused by the size check with the sentence that names its window, which counts as an evaluation error. Before, the text was cut at 6,000 characters whatever the model, so a hand-off full of numbers failed on a 2,048-token local model such as `tev1:0.8b`.

### What `complete` does on a gated phase

With `status:"completed"`:

1. A held phase is refused with the hold, and nothing is asked.
2. A pass the person recorded closes the phase without asking the gate, once the barrier allows: "The person passed this phase's gate, so it completed without asking it."
3. A gate whose attempts are all used is refused at once, and nothing is asked.
4. The barrier runs first, on the stored plan. A phase whose subagents are still working is refused exactly as an ungated one is, and no decision model is asked.
5. The decision model answers every question, or the chat's own model does for a gate that names no model when no decision model may ([A gate needs only a language model](#a-gate-needs-only-a-language-model)). Each question passes when P(yes) is at or above the threshold.
6. The verdict is written in one transaction, which refuses if the gate changed while it was evaluated and, on a pass, checks the barrier and the racing-spawn marks again.

| Verdict | What happens |
|---|---|
| Every question passed | The phase is completed, as an ungated one would be. One attempt is recorded. |
| A question fell short | The phase stays in progress and one attempt is used. The reply lists each failing question with its P(yes) and tells the agent to fix the work (spawn subagents into the phase, or write a hand-off that states the evidence) and complete again. A summary given with `complete` is kept as the phase's hand-off. |
| The last attempt fell short | The phase is blocked. The reply tells the agent to say what the gate asked and what the results lacked, and that only the person can go on: they can give the gate its attempts again, change or remove it, or pass or skip the phase, in the app or with `biorouter session gates`. |
| The gate could not be evaluated | Nothing is decided and no attempt is used: the model was refused, unreachable or too small for the text, it returned uncalibrated probabilities the gate does not accept, or the chat's own model gave an answer that could not be read. The reply says why. A second error in a row blocks the phase. |

`complete` with `status:"blocked"` never asks the gate: "Its gate was not asked, used no attempt and still holds the phases after it". `status:"skipped"` on a gated phase is refused unless the person's skip stands, and a `completed` while the person's skip stands is refused too.

**Who can move a blocked gated phase on:**

| Blocked because | The agent | The person |
|---|---|---|
| The gate did not pass in any of its attempts | Nothing: its completion is refused | Retry, change, remove, pass or skip |
| Two evaluation errors in a row | Complete it again once the model that judges it can be reached (no attempt is used) | The same |
| The agent marked it blocked | Complete it with `status:"completed"` to ask the gate | Change, remove, pass or skip |

### The person's controls

Only the person sets, replaces, removes, passes, skips or retries a gate, from the gate dialog in the app (the chip on a phase row, or "Add gate" on an open phase without one), with `biorouter session gates`, or over HTTP. Each write needs the proof that a person acted on a daemon that holds a user-action key, and the agent's tool has no such action. On a daemon that holds none (`biorouter serve`), reach alone admits it, except on a subagent's chat.

| Control | Route | What it does |
|---|---|---|
| Set or replace | `PUT /sessions/{session_id}/phase-gates/{phase_id}` | The gate becomes the person's, its attempts start over, and a phase its old gate blocked goes back in progress. A model it names is checked, and a model the chat may not use is refused with 422: "… Remove the model from the gate to let the chat's own model judge it. Nothing was changed." A gate that names no model is never refused. |
| Remove | `DELETE /sessions/{session_id}/phase-gates/{phase_id}` | The gate and its record go, and the phases after it are released. |
| Retry | `POST …/decision` with `{"decision": "retry"}` | The attempts and errors start over, a standing pass or skip is dropped, the phase reopens, and the retry is counted. |
| Pass or skip | `POST …/decision` with `"pass"` or `"skip"` | Applied at once, unless a subagent of the phase is still working or a spawn into it is in flight. Then it stands ("Your pass stands: phase "build" completes without its gate once its 2 working subagents finish and the agent completes it.") until the agent completes the phase with the matching status. |

A person's pass or skip is not held by an earlier gate. Each answer is `{message, applied, plan}`, with the plan as `GET /sessions/{session_id}/subagents` shows it, and a refusal is 400 (the phase id is not 1 to 40 lowercase letters, digits or hyphens, or the gate is invalid), 404 (no chat, plan or phase), 409 (a closed phase, or no gate to remove or decide) or 422. Without the proof the answer is 403 with `PHASE_GATES_NEED_USER`.

### How a gate shows

Every evaluation by a decision model is one row of the chat's decision log (origin `gate`, the phase id and title), never a transcript row; a verdict from the chat's own model writes none. The plan's snapshot carries the gate on its phase (`gate`): the threshold, the questions and their count, the attempts used and allowed, the last verdict, the lowest P(yes) at the last verdict, who gave the last verdict (`lastJudge`: `kind` `decision_model` or `chat_model`, the model by its name, never a provider id, and for the chat's model the `reason`), whether the gate has blocked the phase, who set it (`owner`), its model, whether it accepts uncalibrated probabilities, the last error, the person's pass or skip (`personDecision`), the retries the person granted, `needsPerson` and the phases it `holds`. `needsPerson` is true while an open phase is stuck and waits for the person: its gate blocked it, or it was marked blocked. It and `blocked` are never set once the phase is completed or skipped. Each open phase also carries `heldBy`, the gated phase it waits for.

The tool's reply prints the gate as one line under the phase, with "waits for build's gate" on a held phase and "holds …" on the gated one, and the per-turn plan summary marks the phase as gated. A gate whose last check did not pass reads "last not passed (lowest P(yes) 0.42)", never "failed": a gate passes or it does not. The desktop app draws a chip on the phase row (Gate before it is asked, Passed, Not passed, Needs you, Blocked or Skipped) that opens the gate dialog; its tooltip gives the whole sentence, such as the attempt (n of m), "blocked after 3 attempts" when every attempt was used, "could not be evaluated" when evaluation errors in a row blocked it (which use no attempt), or the person's own pass or skip. A held phase reads "Waits for Build's gate". The gate has no row of its own in the chat summary's Decisions section: its chip on the phase is its place. `biorouter session subagents` and the terminal UI's pane mark a passed and a blocked gate as well. A surface reading an older snapshot without the field shows no chip.

## Phase monitors

A phase may also carry `monitors`: at most 4 decision-model monitors, in the shape the `subagent` tool's `monitors` field takes (a yes/no question, a threshold, `stop` or `steer`, and the rest of `MonitorSpec`, described under [Monitors on subagents](decision-models.md#monitors-on-subagents)). Every subagent spawned into the phase afterwards starts with them, as monitors set by the agent. A revision that leaves `monitors` out keeps them, and `[]` clears them for later spawns. Subagents already in the phase keep the monitors they started with, and the reply says so when a revision changes them on a phase that has subagents. A spawn's own monitor replaces a phase monitor with the same id. Each phase monitor's model is checked when the plan adds it: "Phase "build"'s monitor 'Off task' cannot run in this chat: … Nothing was changed."

## Plans from a workflow

A workflow file may declare the plan its chat starts with, as `orchestration_plan` ([workflow schema reference](../workflows/workflow-schema-reference.md#supervision-decision_settings-and-orchestration_plan)). The seed writes it once, into the new chat, before its first message, as the skeleton the file declares: no phase has started and no gate has been asked. Its gates are owned by the workflow, have no 0.50 floor, and bind the agent like any other. A gate that names no model runs for a person with no decision model, judged by the chat's own model; one that names a model the person does not have refuses the workflow, with "Remove the model from the gate to let the chat's own model judge it." A plan needs the Completely Autonomous approval mode, where the agent has the `subagent` and `workspace_orchestrate` tools; started in another mode, the workflow is refused and no chat is left behind. A subagent cannot run phases, so a sub-workflow that declares a plan cannot be spawned.

## A monitor's stop stops the swarm

A plan's subagents are this chat's subagents, so a monitor's stop on the orchestrator stops every one of them that still has work in flight, and the turn waits a bounded time to collect what they had done. A copy of a person's "also watch subagents" monitor that stops one subagent stops its siblings and then the orchestrator. The person's own Stop does the same. The rules are in [A stop stops the chat's subagents](decision-models.md#a-stop-stops-the-chats-subagents).

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
- **The CLI** prints the plan above the subagent list with `biorouter session subagents <session_id>` (`--format json`, `--watch` to redraw every 2 seconds), each gated phase with who set its gate, the person's decision and "needs you", and lists the gates on their own with `biorouter session gates <session_id>`. With no daemon it reads the local store, and a subagent that was running reads as interrupted or unknown.
- **The terminal UI** shows the phases in its subagent pane (`Ctrl-B` to show or hide it, `Ctrl-O` to move focus into it).

A phase's progress counts its subagents as done (completed), running (queued, running or cancelling), blocked, failed (failed, incomplete or interrupted) and quiet. Quiet means a subagent is active but has recorded no activity for 3 minutes. It is a heuristic and is labelled as such, never as stalled.

## Related documentation

- [Subagents](subagents.md): the spawn tool, its parameters, and how a subagent runs in the background.
- [Workspace control](workspace-control.md): delegating, watching and stopping subagents, and the delegation gate.
- [Workspace Control tool reference](workspace-control-tools.md): the exact arguments and refusals of `workspace_orchestrate` and the other workspace tools.
- [Chat summary rail](../desktop-ui/chat-summary-rail.md): where the desktop app draws a chat's subagents and phases.
- [Session metadata contract](session-metadata-contract.md): what makes a session a subagent of a parent.
- [Decision models](decision-models.md): the models a phase gate asks, the decision log its evaluations are written to, and the privacy rule they pass.
- [CLI command reference](../cli/command-reference.md#session-gates): `biorouter session gates`, the person's control over a chat's gates from a terminal.
- [Workflow schema reference](../workflows/workflow-schema-reference.md#supervision-decision_settings-and-orchestration_plan): a plan, with its gates and phase monitors, declared in a workflow file.
- [Decision models in the desktop app](../desktop-ui/decision-models.md#the-phase-gate-dialog): the gate dialog the chip opens.
