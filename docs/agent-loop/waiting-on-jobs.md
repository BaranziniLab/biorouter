# Waiting on jobs

> **What this is.** How the agent waits for a long-running job (a database export, a cluster run, a background shell command) without spending a model request on every status check: the `code_execution__wait_for` tool, the `wait_for()` and `sleep()` functions inside `execute_code` scripts, and the pacing that holds a status check made too soon after the last one.
> **Status:** Current. Code: `crates/biorouter/src/agents/code_execution_extension.rs` and its `wait.rs`, `crates/biorouter/src/tool_monitor/job_status.rs` and `poll_pacing.rs`, and the one call in `ExtensionManager::dispatch_tool_call`.
> **Audience:** people who run long jobs from a chat, extension authors whose tools start jobs, and anyone changing the agent loop's tool dispatch.

## Why waiting costs so much

Every model request sends the whole conversation again. Late in a long chat that is hundreds of thousands of input tokens per request. A model that checks a job by ending its turn, reading one status line and asking again pays that price for every check. In one measured session, 118 of 575 requests were such checks (a script that slept, made one status call and returned to the model), about a quarter of all input tokens. Some were 6 to 10 seconds apart, because the connector suggested polling every 5 seconds while the job was queued.

The fix is to let the host do the waiting. A status check the host makes costs no model tokens. Only the final answer goes back to the model.

## `code_execution__wait_for`

The agent calls this once after starting a job. Biorouter calls the named status tool on an interval, sleeps between checks without involving the model, and returns one short result.

| Argument | Meaning |
|---|---|
| `tool` | The status tool's full name, `server__tool`, for example `cdwagent__CDW-query_job_status`. It must be a tool the chat could import into an `execute_code` script. |
| `arguments` | The status tool's arguments, for example `{"job_id": "abc"}`. Every check sends the same arguments. |
| `until` | When to stop. Omitted or `"inactive"`: when the result no longer reads as an active job. `"changed"`: when the result changes. `{"pointer": "/status", "values": ["completed", "failed"]}`: when the value at that JSON pointer is one of the values. |
| `timeout_seconds` | How long to wait in all. Default 240, at most 280, which stays below the 300 second per-call timeout an MCP extension gets. |
| `min_interval_seconds` | Seconds between checks. Default 30, at least 10. A longer `recommended_poll_seconds`, `retry_after` or `poll_after` in a status result is used instead. |

A result reads as an active job when it carries an `io.biorouter/operation` envelope (see [Operation outcomes](operation-outcomes.md)) whose status is `queued` or `running`, or a `status`, `state` or `phase` field (on the result or on an object directly inside it) such as `queued`, `running`, `executing` or `streaming`. Words such as `completed`, `failed`, `cancelled` and `timed_out` read as finished. The Developer capability's `shell_status` header is read too. With `"changed"`, fields that move with the clock (`elapsed_seconds`, `heartbeat_age`, `eta_seconds`, poll hints and similar) are ignored, so a job that only reports a newer heartbeat has not changed.

The result names how the wait ended, how many checks it made and how long it took, followed by the last status result (cut at 8 KB). The ends are:

- **No longer active**, **changed** or **matched**: the job is done or the condition was met.
- **Not a job**: the first result named no job state. The agent is told to use `"changed"` or a pointer instead.
- **Timed out**: the job is still running. The agent calls `wait_for` again, which costs one model request per wait instead of one per check.
- **Failed**: a status call failed. The result is an error naming the check and the tool's error.
- **Cancelled**: Stop was pressed. The wait ends at once, during a check or a sleep.

Every check goes through the same path as a call an `execute_code` script makes: the boundary refusals, the permission judge, Gate C at dispatch, the secret guard on the arguments and the output redaction. The first check is judged in full; later checks repeat that same admitted call, so they meet the boundary refusals again but not the judge. A person is asked at most once per wait, not once per check. Each check appears in the tool card's executed-calls list.

## Waiting inside a script

Two functions are available inside `execute_code`:

- `wait_for({ tool, arguments, until, timeout_seconds, min_interval_seconds })` does the same wait from a script and returns `{ state, checks, elapsed_seconds, last_result }`. `state` is `finished`, `changed`, `matched`, `not_a_job` or `timed_out`. A failed check throws, like any failed tool call. `tool` must be one of the script's importable tools.
- `sleep(seconds)` pauses the script without spending tokens. One call may sleep at most 280 seconds.

A script may spend at most 600 seconds in `sleep()` and `wait_for()` together. Past that, `sleep()` throws and says to record what the script has and continue with `code_execution__wait_for` or a new script. Stop ends a sleeping or waiting script at once.

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
- **What the model sees.** A held direct call carries one extra line in its result, saying it was held and pointing at `code_execution__wait_for`. A held call inside a script carries nothing extra in the value the script receives; the script's own result says once how many of its checks were held and for how long. `wait_for`'s own checks are never held, because it spaces them itself, but they are remembered, so a direct check made right after `wait_for` returns is spaced from its last check.
- **Where it runs.** In `ExtensionManager::dispatch_tool_call`'s future, after every admission check and around the existing call (`call_paced_tool`), so it covers the agent loop, `POST /agent/call_tool` and every call a script makes. It adds no dispatch path and reads no privacy state. Code Execution's own tools are exempt.

The records live in memory only, keyed by chat id, tool name and a hash of the arguments, and expire after an hour.

## For extension authors

A job tool helps the host most when its status result:

- carries the `io.biorouter/operation` envelope, or at least a job id and a `status` field using the words above;
- suggests a realistic `recommended_poll_seconds` (30 seconds or more while a job is queued or running, not 5);
- puts the job's final details (row count, output path, error) in the finished result, since that is what `wait_for` hands back.

## Related documentation

- [Operation outcomes](operation-outcomes.md)
- [Long-running tasks](long-running-tasks.md)
- [Turn cancellation and process reaping](turn-cancellation-and-process-reaping.md)
- [Code Execution](../extensions/built-in/code-execution.md)
