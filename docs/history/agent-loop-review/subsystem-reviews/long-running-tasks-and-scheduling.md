# Long-running tasks, background processes and scheduling — architecture review

> **What this is.** One of ten subsystem reviews from the 2026-07 BioRouter agentic-loop review. It documents the four mechanisms for work that outlives a single tool call — background shell jobs, subagents, the cron scheduler and MCP elicitation — plus process tracking and what survives a daemon restart, and records eleven gaps.
> **Status:** Historical record — a snapshot of the code *before* the agent-loop fix campaign, whose findings were then implemented. Gaps #1 and #3 (no PID-file reaping, a dead-coded `list()`) were fixed by BR-37, BR-38 and BR-42 plus the `shell_list` surfacing proposal; scheduler reconciliation by BR-39 and BR-40; and blocking subagents by BR-41 (`agents/subagent_handle.rs`). Its praise for the background-job design still reads true.
> **Audience:** developers working on background execution, subagents, or the scheduler.

The subsystem question is how BioRouter (CLI, the `biorouterd` daemon, and the Electron GUI) handles work that does not finish inside one blocking tool call. Identifier key: `BR-NN` are proposal ids from the [master improvement-proposal list](../improvement-proposals.md); the numbered items under "Gaps and weaknesses" are what sibling reviews cite as `long-running.md gap #N` (the file's former name).

> **Note.** The review recorded no commit or branch, and line numbers in the citations below have drifted since; all paths are repository-relative and should be treated as pointers to the right function, not exact locations.

## Overview

There are four largely independent mechanisms for "work that does not finish in
one blocking call":

1. **Background shell jobs** (developer MCP extension). A `shell` call with
   `background=true` spawns the command in its own process group, returns a
   `job_id` immediately, and the agent later reads incremental output / waits /
   kills via `shell_output`, `shell_wait`, `shell_kill`. State lives in an
   in-memory `BackgroundJobs` registry inside the `DeveloperServer` instance.
2. **Subagents** (`subagent` tool). Delegates a task to a child `Agent` with its
   own context/session; the parent's tool call blocks until the child finishes
   and returns its final message as the tool result. Concurrency is bounded by a
   global semaphore + in-flight ceiling.
3. **Cron scheduler** (`Scheduler`). Durable `ScheduledJob`s persisted to
   `schedule.json`, fired by `tokio_cron_scheduler`, each firing running a
   workflow as its own `SessionType::Scheduled` session. `/loop` and `/schedule`
   slash commands are thin wrappers over it.
4. **Action-required / elicitation** (`ActionRequiredManager`). Lets an MCP
   server pause a tool call mid-run to ask the user a structured question; the
   answer arrives as a follow-up `/reply`.

The sections below take those four mechanisms in that order — background shell jobs,
subagents, the scheduler, then elicitation — and add two cross-cutting questions:
how spawned processes are tracked (immediately after the background-jobs section,
because that is where the registries live) and what survives a daemon restart.

Text data-flow for a background shell job:

```text
agent → shell{background:true} → BackgroundJobs.spawn()
    → tokio::process::Command (own process group, kill_on_drop=false)
    → supervisor task: child.wait() → records JobStatus::Exited(code)
    → reader tasks: stdout/stderr → shared Output buffer (cursor-tracked)
job_id returned immediately
...later...
agent → shell_wait{job_id} → parks on watch::Receiver<bool> (bounded timeout)
agent → shell_output{job_id} → drains only-new output since last cursor
agent → shell_kill{job_id} → SIGTERM(-pgid) then SIGKILL
```

Text data-flow for a scheduled run:

```text
tokio_cron_scheduler fires → claim_run_slot() (overlap/pause/rate-limit/max_runs guard)
    → persist_jobs(schedule.json) → execute_job()
    → load workflow YAML/JSON → Agent::new() → create SessionType::Scheduled session
    → agent.reply(prompt) stream drained to completion
    → SessionEnd hooks → persist last_run / clear currently_running
```

## Review questions answered

### Background execution versus blocking

Both exist, and the split is explicit.

**Foreground shell blocks the turn.** A normal `shell` call runs to completion
inside the tool call: `execute_shell_command` spawns the child, streams
stdout/stderr line-by-line to the client, and `select!`s the output future
against a cancellation token
(`crates/biorouter-mcp/src/developer/rmcp_developer.rs:1327-1352`). The tool
result is only returned after `child.wait()` returns. So a long command blocks
the turn unless cancelled.

**Background execution with later retrieval exists and is well-built.** Setting
`background=true` routes to `BackgroundJobs::spawn`
(`rmcp_developer.rs:1122-1131`), which returns a `job_id` immediately and keeps
the process alive across tool calls. The implementation lives in
`crates/biorouter-mcp/src/developer/background.rs`:

- The job is spawned with `kill_on_drop(false)` so it survives the tool call
  returning (`background.rs:119-125`), reusing the hardened foreground command
  builder (own process group, sanitized git/editor env).
- A **supervisor task** owns the `Child` and records the terminal status from
  the real OS exit code — never from log text (`background.rs:144-159`). This is
  the source-of-truth design the module comment calls out as mirroring Claude
  Code / Codex.
- Output is captured by reader tasks into a shared buffer with a **per-job read
  cursor**, so `shell_output` returns only new bytes since the last check
  (`drain_new_output`, `background.rs:186-195`), capped at 400 KB
  (`MAX_OUTPUT_BYTES`, `background.rs:26`).
- `shell_wait` is a **bounded in-turn watch**: it parks race-free on a
  `watch::Receiver<bool>` that flips when the job reaches terminal state, with a
  default 120 s / max 600 s timeout, returning `status: running` (job NOT killed)
  on timeout (`background.rs:215-234`, `DEFAULT_WAIT_SECS`/`MAX_WAIT_SECS`
  `background.rs:28-29`). This lets the agent wait without busy-looping or ending
  the turn.
- `shell_kill` signals the whole process group SIGTERM then SIGKILL 1.5 s later
  (`kill_process_group`, `background.rs:295-311`).

The four tools are surfaced in `rmcp_developer.rs`: `shell` (1103), `shell_wait`
(1179), `shell_output` (1200), `shell_kill` (1216). The tool descriptions
explicitly instruct the model to use `background=true` instead of appending `&`
(`rmcp_developer.rs:1101`, `194-200`).

### How spawned processes are tracked — registries, PIDs, orphan cleanup

Three separate registries, none unified:

1. **Foreground shell**: `running_processes: Arc<RwLock<HashMap<String,
   CancellationToken>>>` keyed by MCP request id
   (`rmcp_developer.rs:311-313`, insert 1136-1139, remove 1146-1157). It stores
   cancellation tokens, not PIDs, so foreground commands can be cancelled but the
   map is a transient in-turn structure.

2. **Background shell jobs**: `BackgroundJobs { jobs: Mutex<HashMap<String,
   Arc<Job>>>, next_id }` (`background.rs:90-94`). Each `Job` records the
   process-group leader PID, status, output buffer, a `killed` flag, and a
   `done_rx` watch (`background.rs:68-86`). IDs are monotonically increasing
   (`job-1`, `job-2`, …). Crucially this registry is **created per
   `DeveloperServer` instance** (`rmcp_developer.rs:704`) and lives only in
   memory.

3. **Scheduler running tasks**: `running_tasks: Arc<Mutex<HashMap<String,
   CancellationToken>>>` keyed by schedule id (`scheduler.rs:233`), used by
   `kill_running_job` (`scheduler.rs:753-776`).

**Orphan cleanup: mostly absent for these three.** There is no PID-file /
parent-death reaping for background shell jobs or scheduled runs. Background jobs
deliberately set `kill_on_drop(false)`, so if the daemon crashes, the spawned
process group is orphaned with no recovery mechanism. Contrast this with the
llama.cpp sidecar, which *does* implement proper orphan reaping via
`<data>/llamacpp/run/<ppid>.pid` files and a `reap_orphans()` swept on each
`ensure()` (`crates/biorouter/src/providers/llamacpp_sidecar.rs:833-936,1102`).
That pattern was not applied to the shell background jobs. The only
process-group hygiene is that jobs run in their own process group so a *live*
`shell_kill` can take down the whole tree.

### How subagents work — spawning, task config, result return, parallelism

**Spawning.** The `subagent` tool (`crates/biorouter/src/agents/subagent_tool.rs`)
accepts `instructions` (ad-hoc), `subworkflow` (predefined), `parameters`,
`extensions`, `settings` (provider/model/temperature override), and `summary`
(`SubagentParams`, subagent_tool.rs:81-101). `handle_subagent_tool` builds a
`Workflow` then calls `execute_subagent` (subagent_tool.rs:290).

**Task config.** `TaskConfig` (`agents/subagent_task_config.rs`) carries the
provider, parent session id, parent working dir, inherited extensions, and
`max_turns` (default 25, overridable via `BIOROUTER_SUBAGENT_MAX_TURNS`,
subagent_task_config.rs:9-53). `execute_subagent` creates a fresh
`SessionType::SubAgent` session (subagent_tool.rs:319-331), applies
settings/extension overrides (subagent_tool.rs:460-499 — empty `extensions:[]`
means none, omitted means inherit all), then calls
`run_complete_subagent_task`.

**Execution and result return.** `run_complete_subagent_task`
(`agents/subagent_handler.rs:30`) builds an `Arc<Agent>`, fires a `SubagentStart`
hook (observe-only, subagent_handler.rs:139-154), sets provider + extensions,
renders `subagent_system.md`, and drives `agent.reply(...)` to completion,
collecting the conversation. Result return has two modes: if the workflow has a
response schema, the child's `final_output_tool` value is returned
(subagent_handler.rs:236-245); else with `summary=true` (default) only the
**last text message** is returned, otherwise all text + tool-result text is
concatenated (subagent_handler.rs:58-114). A `SUMMARY_INSTRUCTIONS` block is
appended to the child's instructions when `summary=true`
(subagent_tool.rs:70-79, 376-379). The parent's `subagent` tool call **blocks**
until the child returns — the tool result is the child's summary.

**Parallelism limits.** Two caps guard against fork-bombs
(subagent_tool.rs:26-68, 298-317): a global `SUBAGENT_SEMAPHORE` throttling
*concurrent* subagents (default 8, `BIOROUTER_SUBAGENT_MAX_CONCURRENT`), and an
`SUBAGENT_INFLIGHT` atomic ceiling on queued+running (default 64,
`BIOROUTER_SUBAGENT_MAX_INFLIGHT`) that hard-refuses new spawns with an
`INVALID_PARAMS` error. Parallelism is model-driven: "make multiple `subagent`
tool calls in the same message" (subagent_tool.rs:159-160). Note
`sequential_when_repeated` is only a **description hint**, not enforced
(subagent_tool.rs:224-226) — the LLM controls sequencing. There is also a
separate `subagent_execution_tool` module, but it currently only defines
notification event types (`TaskStatus`, `TaskExecutionNotificationEvent`) and a
`list()` helper — no live task-execution engine is wired up
(`subagent_execution_tool/mod.rs`, `notification_events.rs`).

### The cron scheduler, and how scheduled runs differ from interactive ones

`Scheduler` (`crates/biorouter/src/scheduler.rs`) wraps
`tokio_cron_scheduler::JobScheduler`. Jobs are `ScheduledJob` structs
(scheduler.rs:139-161) persisted as JSON to `schedule.json` in the data dir
(`get_default_scheduler_storage_path`, scheduler.rs:67-71). Each job is turned
into a cron task via `create_cron_task` (scheduler.rs:267-360), which normalises
5-field cron to 6-field (prepending `0` seconds, scheduler.rs:273-291) and fires
in the machine's local timezone (`Job::new_async_tz`, scheduler.rs:295).

Every firing goes through `claim_run_slot` (scheduler.rs:170-211) under one lock,
which skips when the job is gone, paused, still running from a prior firing
(**overlap guard** so a slow run never stacks), or has hit `max_runs` (which
auto-pauses). It additionally **defers** (without consuming a run) when the
provider is rate-limited (`is_rate_limited()`) or a user is mid-conversation
(`interactive_active()` + `pause_on_active()`, scheduler.rs:192-202) — a "pause
when active" idea so background work never competes with the user for provider
budget. On a real run it stamps `last_run`, `currently_running`,
`process_start_time`, bumps `run_count`, and persists.

`execute_job` (scheduler.rs:796-929) loads the workflow file, builds a fresh
`Agent`, creates a `SessionType::Scheduled` session, drains the reply stream to
completion, fires a `SessionEnd` hook, and records the session id.

**Differences from interactive runs:**
- Session type is `Scheduled` (scheduler.rs:844) vs `User`; sessions are tagged
  with `schedule_id` so `/schedule sessions <id>` can list them
  (scheduler.rs:589-610).
- A brand-new `Agent` + provider is created per firing from global config
  (scheduler.rs:823-847) — no in-memory conversation continuity; each firing is
  a cold start over the workflow prompt.
- Interactive `/reply` holds an `InteractiveTurnGuard`
  (`biorouter-server/src/routes/reply.rs:261`) which increments the counter that
  causes the scheduler to *defer* firings (scheduler.rs:34-54).
- `/loop` vs `/schedule`: both wrap a one-prompt workflow file
  (`agents/recurring.rs:182-231`). `/loop` uses a `loop-` id prefix and a
  `max_runs` cap (default 100, `BIOROUTER_LOOP_MAX_RUNS`, recurring.rs:32-41) so
  it auto-stops; `/schedule` uses `task-` and is unbounded/durable
  (recurring.rs:25-32, 388-391). `/schedule run <id>` fires immediately in a
  detached `tokio::spawn` (recurring.rs:435-447).
- If no scheduler service is injected (plain CLI/TUI), a lazily-created
  in-process `Scheduler` over default storage is used
  (`Agent::scheduler`, recurring.rs:162-177). In the daemon/GUI the
  `AgentManager` owns a single shared `Scheduler`
  (`execution/manager.rs:32,70`), which also installs a first-run "Daily
  Meditation" schedule for the Soul KB (`execution/manager.rs:49`).

### Mid-run user decisions — `action_required_manager.rs`

`ActionRequiredManager` (`crates/biorouter/src/action_required_manager.rs`) is
the **MCP elicitation** path — how an extension server can pause a tool call
mid-run to ask the user a structured question. It is a global singleton
(`global()`, lines 33-37) holding a `pending` map of oneshot senders and an
`mpsc` channel of outbound request messages (lines 17-31).

Flow: an MCP client's `create_elicitation` handler calls
`request_and_wait(message, schema, timeout)` (`agents/mcp_client.rs:254-285`,
300 s timeout). This registers a pending oneshot, pushes an
`action_required_elicitation` assistant message into the mpsc channel
(action_required_manager.rs:39-79), then blocks the tool call on the oneshot with
a timeout. The agent drains these messages from the channel into the reply
stream and persists them (`Agent::drain_elicitation_messages`,
`agents/agent.rs:404-415`). The user answers via a follow-up `/reply` carrying an
`ElicitationResponse`; `Agent::reply` detects it and calls `submit_response`,
which fires the oneshot and unblocks the parked tool call
(`agents/agent.rs:1248-1269`, action_required_manager.rs:81-98).

Important distinction: this is separate from **tool-permission confirmation**,
which is a different mechanism — `agent.handle_confirmation` driven by the
`/action-required/tool-confirmation` route
(`biorouter-server/src/routes/action_required.rs:33-56`). Both are "mid-run user
decisions" but only elicitation goes through `ActionRequiredManager`.

### What survives a daemon restart

- **Scheduled jobs**: yes. `schedule.json` is persisted and reloaded on startup
  (`load_jobs_from_storage`, scheduler.rs:481-549). Managed `/loop` and
  `/schedule` workflow files live under `scheduled_workflows/`.
- **Sessions / conversation history**: yes, persisted to SQLite by the
  `SessionManager` (per the architecture notes; scheduled/subagent sessions are
  written there too).
- **Secrets/config**: yes (OS keychain / config.yaml).
- **Background shell jobs**: NO. `BackgroundJobs` is in-memory per
  `DeveloperServer` (`rmcp_developer.rs:704`, `background.rs:90-94`). The
  registry is lost and the orphaned OS processes are never reaped.
- **Subagents**: NO. Purely in-memory; an in-flight subagent is lost on restart
  (its session row persists, but the run does not resume).
- **In-progress scheduled runs**: NOT recovered correctly. `load_jobs_from_storage`
  inserts each job verbatim without resetting `currently_running`
  (scheduler.rs:512-548). A job that was mid-run at crash time reloads with
  `currently_running: true` and is then **permanently skipped** by the overlap
  guard in `claim_run_slot` (scheduler.rs:175-178) — a stuck-job bug on crash.
- **Pending elicitations**: NO. `ActionRequiredManager`'s pending oneshots are
  in-memory (action_required_manager.rs:17-21); a restart drops them and any
  parked tool call is gone.

## Notable design choices (worth keeping)

- **Exit-status truth for background jobs.** Done-vs-running is decided by the OS
  exit code recorded by a supervisor task, never by scraping log text
  (`background.rs:144-159`). This is the correct, state-of-the-art shape and
  matches Claude Code / Codex.
- **Cursor-based incremental output.** `shell_output` returns only new bytes
  since the last read (`background.rs:186-195`), which keeps token usage bounded
  when polling a chatty process.
- **Race-free bounded wait.** `shell_wait` parks on a `watch` channel with a
  timeout and explicitly does not kill on timeout (`background.rs:215-234`) — the
  ergonomics Claude Code uses.
- **Resource-aware scheduler deferral.** Skipping firings while rate-limited or
  while the user is interacting (scheduler.rs:192-202) is a genuinely good idea
  for a personal research agent that shares one provider budget.
- **Fork-bomb guards on subagents** (semaphore + in-flight ceiling,
  subagent_tool.rs:26-68) — recursive subagent spawning is otherwise unbounded.
- **Overlap guard + max_runs auto-pause** so slow scheduled runs never stack and
  `/loop` cannot run forever (scheduler.rs:170-211).

## Gaps and weaknesses

These eleven items fed the improvement phase. They are what other documents in this
review cite as `long-running.md gap #N`; the numbering below is that scheme and is stable.

1. **Background jobs don't survive restart and orphan on crash.** They set
   `kill_on_drop(false)` (`background.rs:119`) but there is no PID-file /
   parent-death reaping like the llama.cpp sidecar already has
   (`llamacpp_sidecar.rs:833-936`). A daemon crash leaves detached process groups
   running forever with no way to discover or kill them. Reuse the sidecar's
   `run/<ppid>.pid` reaping pattern.
2. **Stuck scheduled jobs after a crash.** `load_jobs_from_storage` never resets
   `currently_running`/`current_session_id`/`process_start_time`
   (scheduler.rs:512-548), so a job running at crash time is permanently skipped
   by the overlap guard. A one-line reconcile on load (force `currently_running =
   false`) would fix it.
3. **No cross-turn/persistent handle to background jobs from the model's world
   model.** `job_id`s are ephemeral in-memory ints; there is no `shell_list`
   surfaced as a tool (`list()` exists but is `#[allow(dead_code)]`,
   `background.rs:251`). If the agent forgets a `job_id` mid-session it cannot
   enumerate what it started. State-of-the-art agents expose a "list background
   tasks" surface.
4. **Subagents are fully blocking / no async subagent handle.** The parent tool
   call blocks until the child finishes (subagent_tool.rs:341-349). There is no
   "spawn subagent, get a handle, poll later" model, so a long subagent stalls
   the parent turn. Parallelism only comes from issuing many blocking calls in
   one message, all of which must complete before the turn advances.
5. **Subagent results are lossy.** Default `summary=true` returns only the last
   text message (subagent_handler.rs:58-70); if the child ends on a tool call or
   empty message the parent gets "No text content in last message". No structured
   status/error envelope unless a response schema is defined.
6. **`sequential_when_repeated` is advisory only** (subagent_tool.rs:224-226) —
   the runtime cannot actually enforce ordering; a misbehaving model can run
   "sequential-only" subworkflows in parallel.
7. **Cold-start scheduled runs.** Each firing builds a brand-new agent from
   global config (scheduler.rs:823-847). There is no memory/context carryover
   between `/loop` iterations, so a polling loop cannot cheaply remember "what I
   saw last time" except via external side effects (files, KB).
8. **Scheduler tied to process lifetime.** Jobs only fire while a BioRouter
   process is alive (documented to the user, recurring.rs:337-339). There is no
   OS-level scheduling (launchd/systemd/cron) fallback, so "durable" `/schedule`
   silently does nothing when the app is closed.
9. **Two unrelated `kill_process_group` implementations** (shell.rs:148 and
   background.rs:295) and three separate process/task registries with no shared
   abstraction — duplication that will drift.
10. **Elicitation is global, not session-scoped.** `ActionRequiredManager` is a
    process-wide singleton with a single shared `request_rx`
    (action_required_manager.rs:17-21; drained in agent.rs:407). With concurrent
    sessions, elicitation request routing depends on whichever agent drains the
    channel first; there is no per-session addressing, and a 300 s hard timeout
    (mcp_client.rs:271) silently fails the tool.
11. **No global "long-running work" dashboard.** Background shell jobs,
    subagents, and scheduled runs are three disjoint systems with no unified view
    of "what is this agent currently running," which is what mature coding agents
    expose to the user.

## Related documentation

- [Core agent loop and tool dispatch](core-loop-and-tool-dispatch.md) — the turn these mechanisms hang off, including the per-extension timeout that bounds a blocking subagent call.
- [Execution and verification compared with other agents](../competitive-comparison/execution-and-verification.md) — how this background-job and subagent design measures against nine other coding agents.
- [Subagents guide](../../../agent-loop/subagents.md) — the current, living reference for the `subagent` tool.
- [Scheduled jobs guide](../../../workflows/scheduled-jobs.md) — the current, living reference for `/schedule` and `/loop`.
- [Wave 1 processes report](../../agent-loop-campaign/wave-reports/wave-1-processes.md) — what was actually built in response to these gaps.
