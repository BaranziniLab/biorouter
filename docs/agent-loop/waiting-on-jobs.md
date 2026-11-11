# Waiting on jobs

> **What this is.** How the agent waits for a long-running job (a database export, a cluster run, a background shell command) without spending a model request on every status check: the `code_execution__wait_for` tool, the `code_execution__watch_job` tool that ends the turn and resumes the chat when the job finishes, the `wait_for()` and `sleep()` functions inside `execute_code` scripts, and the pacing that holds a status check made too soon after the last one.
> **Status:** Current. Code: `crates/biorouter/src/agents/code_execution_extension.rs` and its `wait.rs` and `watch.rs`, `crates/biorouter/src/tool_monitor/job_status.rs` and `poll_pacing.rs`, the one call in `ExtensionManager::dispatch_tool_call`, and the `job_watch` kind in `crates/biorouter-mcp/src/active_work.rs`.
> **Audience:** people who run long jobs from a chat, extension authors whose tools start jobs, and anyone changing the agent loop's tool dispatch.

## Why waiting costs so much

Every model request sends the whole conversation again. Late in a long chat that is hundreds of thousands of input tokens per request. A model that checks a job by ending its turn, reading one status line and asking again pays that price for every check. In one measured session, 118 of 575 requests were such checks (a script that slept, made one status call and returned to the model), about a quarter of all input tokens. Some were 6 to 10 seconds apart, because the connector suggested polling every 5 seconds while the job was queued.

The fix is to let the host do the waiting. A status check the host makes costs no model tokens. Only the final answer goes back to the model.

## Which wait to use

| The job takes | Use | Model requests while it runs |
|---|---|---|
| Seconds to a few minutes, and the next step needs the result | `wait_for()` inside the script that started the job, or `code_execution__wait_for` | None until the job ends; one more per 10 minutes of waiting |
| Longer than a few minutes, or an unknown time | `code_execution__watch_job`, then end the turn | None. The chat is resumed once, when the job ends |
| A loop the script controls (several jobs, a custom condition) | `sleep()` and a status call inside one script | None, for up to 600 seconds per script |

A request made to check a job is the expensive way, whatever the interval: it re-reads the whole conversation to learn one status word.

## `code_execution__wait_for`

The agent calls this once after starting a job. Biorouter calls the named status tool on an interval, sleeps between checks without involving the model, and returns one short result.

| Argument | Meaning |
|---|---|
| `tool` | The status tool's full name, `server__tool`, for example `cdwagent__CDW-query_job_status`. It must be a tool the chat could import into an `execute_code` script. |
| `arguments` | The status tool's arguments, for example `{"job_id": "abc"}`. Every check sends the same arguments. |
| `until` | When to stop. Omitted or `"inactive"`: when the result no longer reads as an active job. `"changed"`: when the result changes. `{"pointer": "/status", "values": ["completed", "failed"]}`: when the value at that JSON pointer is one of the values. |
| `timeout_seconds` | How long to wait in all. Default 240, at most 600. |
| `min_interval_seconds` | Seconds between checks. Default 30, at least 10. A longer `recommended_poll_seconds`, `retry_after` or `poll_after` in a status result is used instead. |

A result reads as an active job when it carries an `io.biorouter/operation` envelope (see [Operation outcomes](operation-outcomes.md)) whose status is `queued` or `running`, or a `status`, `state` or `phase` field (on the result or on an object directly inside it) such as `queued`, `running`, `executing` or `streaming`. Words such as `completed`, `failed`, `cancelled` and `timed_out` read as finished. The Developer capability's `shell_status` header is read too. With `"changed"`, fields that move with the clock (`elapsed_seconds`, `heartbeat_age`, `eta_seconds`, poll hints and similar) are ignored, so a job that only reports a newer heartbeat has not changed.

The result names how the wait ended, how many checks it made and how long it took, followed by the last status result (cut at 8 KB). The ends are:

- **No longer active**, **changed** or **matched**: the job is done or the condition was met.
- **Not a job**: the first result named no job state. The agent is told to use `"changed"` or a pointer instead.
- **Timed out**: the job is still running. The agent calls `wait_for` again, which costs one model request per wait instead of one per check, or sets a watch with `watch_job` and ends its turn.
- **Failed**: a status call failed. The result is an error naming the check and the tool's error.
- **Cancelled**: Stop was pressed. The wait ends at once, during a check or a sleep.

**Why ten minutes.** The 300 second per-call timeout (`DEFAULT_EXTENSION_TIMEOUT`) is enforced on the transport of an MCP extension (`mcp_client::await_response`). Code Execution is an in-process platform extension, and `ExtensionManager::dispatch_tool_call` adds no timeout of its own, so a `wait_for` call may block longer than 300 seconds. Each check it makes is an ordinary short call to the status tool and stays under that limit. The cap stays at ten minutes because the turn is open while it waits: a message the person sends meanwhile waits too (Stop ends the wait at once). A coding-agent bridge (Claude Code or Codex calling Biorouter's tools) has its own tool timeout, often 300 seconds, so a bridged child should keep the default. Longer jobs belong to `watch_job`.

Every check goes through the same path as a call an `execute_code` script makes: the boundary refusals, the permission judge, Gate C at dispatch, the secret guard on the arguments and the output redaction. The first check is judged in full; later checks repeat that same admitted call, so they meet the boundary refusals again but not the judge. A person is asked at most once per wait, not once per check. Each check appears in the tool card's executed-calls list.

## `code_execution__watch_job`

A watch is assigned by the agent and fires on its own. The agent calls `watch_job` after starting a job and then ends its turn. Biorouter checks the job in the background and resumes the chat once when the job ends. No model request is made while the job runs, however long it takes.

| Argument | Meaning |
|---|---|
| `tool`, `arguments`, `until` | As for `wait_for`. |
| `min_interval_seconds` | The first background check comes after at least this long, and never sooner than 30 seconds. |
| `max_wait_minutes` | How long to watch. Default 60, at most 240. |
| `note` | What to do when resumed, at most 500 characters. It comes back with the job's last status. |
| `cancel` | A watch id such as `watch-3`, or `all`, to cancel this chat's watch instead of starting one. |

What happens:

1. **One check now.** `watch_job` checks the job once, in the turn, admitted in full like `wait_for`'s first check (boundary refusals, the permission judge, Gate C, the secret guard and output redaction). A job that is already done is answered at once and no watch is set. A result with no job state is answered as for `wait_for`.
2. **A watch id.** Otherwise the result names the watch (`watch-3`), says "End your turn; this chat resumes automatically when the job finishes.", and echoes the current state in short. Watching the same tool with the same arguments again returns the same watch. A chat may hold 8 watches, the process 64.
3. **Checks in the background.** The first background check comes after the longer of 30 seconds and the job's own hint (`recommended_poll_seconds`, `retry_after`, `poll_after`). Each later interval is half again as long, up to 5 minutes, and a longer hint is honoured up to 15 minutes. Each check repeats the admitted call: it samples the chat's capability afresh from the model bound now, re-runs the boundary refusals (never the judge, so a person is asked at most once per watch), and dispatches through `ExtensionManager::dispatch_tool_call`, where Gate C, the private-extension ratchet and the output redaction apply. Three failed checks in a row end the watch.
4. **One wake.** When the job is no longer active (or the result changed or matched), when the checks keep failing, or when the time limit passes, the chat receives one message of at most about 2,000 characters: `[watch watch-3] <tool>: <how it ended> (<checks> over <time>)`, the last status with its timing fields left out, and the agent's note. The message is framed as injected text from "Biorouter job watch", since the status it quotes came from an extension, and it never counts as the person typing (it does not buy the turn extra steps past a safety limit).

How the chat is resumed:

- **A turn is running**: the message is steered into it, at the next safe boundary (see [Steering a running turn](steering-a-running-turn.md)).
- **The chat is idle**: a turn starts on the message, through the same detached-turn runner `workspace_send_prompt` uses. The turn runs on the chat's own bound model under its own privacy tier, and its message is ordinary activity, so the chat moves in the sidebar like any other reply.
- **The chat is paused by a usage limit**: no turn starts. The message is stored with the limit's notice and the words "This chat is paused by a usage limit, so the watch did not resume it." (See [Usage limits](usage-limits.md).)
- **The chat asks before running tools and no Biorouter window is open** to show the card: the message is stored as a note and says to send any message to continue, so no approval waits unseen.
- **A turn is closing** and still holds the lock: the wake retries for a few seconds, then stores a note.

How a watch ends without the job:

| Cause | What the chat gets |
|---|---|
| The person cancels it from the active work list (`POST /active_work/{id}/cancel`, kind `job_watch`) | A note saying the watch was cancelled and the chat will not be resumed for it |
| Stop is pressed during the turn that set it | A note saying Stop ended the watch |
| The agent calls `watch_job` with `cancel` | Nothing more; the tool result said so |
| The chat's agent is stopped (`POST /agent/stop`) | A note saying the watch ended because the chat was stopped |
| The chat is deleted | Nothing. The watch compares the chat's incarnation before every check and before the wake, so it never checks or writes for a deleted chat or for a later chat that reuses the id |
| Biorouter restarts | Nothing: watches live in memory only, and the tool result says so when the watch is set |

Stop pressed on a later turn does not end a watch; cancel it from the active work list or ask the agent to. The active work row names the chat, so `GET /active_work` shows it only to a caller that could open that chat.

`watch_job` needs the Biorouter app or daemon, which owns the turn lock and the detached-turn runner. Without one (the CLI talking to the session store directly) it refuses and points at `wait_for`.

## Waiting inside a script

Two functions are available inside `execute_code`:

- `wait_for({ tool, arguments, until, timeout_seconds, min_interval_seconds })` does the same wait from a script and returns `{ state, checks, elapsed_seconds, last_result }`. `state` is `finished`, `changed`, `matched`, `not_a_job` or `timed_out`. A failed check throws, like any failed tool call. `tool` must be one of the script's importable tools.
- `sleep(seconds)` pauses the script without spending tokens. One call may sleep at most 280 seconds.

A script may spend at most 600 seconds in `sleep()` and `wait_for()` together. Past that, `sleep()` throws and says to record what the script has and continue with `code_execution__wait_for` or a new script. The `execute_code` description points longer jobs at `code_execution__watch_job`. Stop ends a sleeping or waiting script at once. While it sleeps or waits, a script hands back the tool-dispatch slot its `execute_code` call holds (the daemon runs at most eight tool calls at once, across every chat), so sleeping scripts never hold up other tool calls; it takes the slot back before it carries on.

```javascript
import { submit_job, job_status } from "module_name";
const job = submit_job({ query: "..." });
const done = wait_for({ tool: "module_name__job_status", arguments: { job_id: job.job_id } });
record_result({ state: done.state, status: done.last_result });
```

```javascript
let status = job_status({ job_id: id });
while (status.status === "running" || status.status === "queued") {
  sleep(30);
  status = job_status({ job_id: id });
}
record_result(status);
```

The `execute_code` and `wait_for` tool descriptions say this plainly: every model request re-reads the whole conversation, so the agent waits inside one script or with `wait_for`, and never returns to the model just to check on a job.

## Pacing repeated status checks

Guidance alone does not stop a model that checks too often, so the host also spaces repeated checks. When the same call (same chat, same tool, same arguments) last returned an identified active job and is dispatched again before its interval has passed, the call is held until the interval has passed, then runs. It is never refused.

- **Identified active job** is the repetition guard's own reading: a job id (`job_id`, `operation_id`, `task_id`, `run_id` and similar) beside an active `status`, `state` or `phase`, an active `io.biorouter/operation` envelope, or a running `shell_status` header. A result that merely contains the word "running" is not paced.
- **The interval** is the larger of the configured minimum and the job's own `recommended_poll_seconds`, `retry_after` or `poll_after`. A connector's 5 second hint does not shorten the minimum.
- **The minimum** is `BIOROUTER_STATUS_CHECK_MIN_INTERVAL_SECONDS` in `config.yaml`, default 30. `0` turns pacing off.
- **A hold** is at most 120 seconds and ends at once on Stop.
- **What the model sees.** A held direct call carries one extra line in its result, saying it was held and pointing at `code_execution__wait_for` and `code_execution__watch_job`. A held call inside a script carries nothing extra in the value the script receives; the script's own result says once how many of its checks were held and for how long. `wait_for`'s and a watch's own checks are never held, because they space themselves, but they are remembered, so a direct check made right after `wait_for` returns is spaced from its last check.
- **Where it runs.** In `ExtensionManager::dispatch_tool_call`'s future, after every admission check and around the existing call (`call_paced_tool`), so it covers the agent loop, `POST /agent/call_tool` and every call a script makes. It adds no dispatch path and reads no privacy state. Code Execution's own tools are exempt.

The records live in memory only, keyed by chat id, tool name and a hash of the arguments, and expire after an hour.

## For extension authors

A job tool helps the host most when its status result:

- carries the `io.biorouter/operation` envelope, or at least a job id and a `status` field using the words above;
- suggests a realistic `recommended_poll_seconds` (30 seconds or more while a job is queued or running, not 5);
- puts the job's final details (row count, output path, error) in the finished result, since that is what `wait_for` hands back and what a watch's wake message quotes (its top-level fields first, when the result is long).

## Related documentation

- [Operation outcomes](operation-outcomes.md)
- [Steering a running turn](steering-a-running-turn.md)
- [Usage limits](usage-limits.md)
- [Long-running tasks](long-running-tasks.md)
- [Turn cancellation and process reaping](turn-cancellation-and-process-reaping.md)
- [Code Execution](../extensions/built-in/code-execution.md)
- [Todo capability: how progress reaches the checklist](../extensions/built-in/todo.md#how-progress-reaches-the-checklist)
