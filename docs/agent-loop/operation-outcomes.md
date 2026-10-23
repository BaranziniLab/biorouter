# Asynchronous operation outcomes

A status lookup can succeed while the operation it describes has failed. BioRouter keeps these facts separate: `isError` describes the tool call; the operation envelope describes the asynchronous job. A successful lookup of a failed job keeps `isError: false`.

## Producer contract

Return this envelope in the MCP `CallToolResult._meta` object, under the exact key `io.biorouter/operation`:

```json
{
  "version": 1,
  "id": "synthetic-job",
  "status": "failed",
  "error": {
    "code": "query_failed",
    "kind": "tool_failure",
    "message": "Synthetic query could not complete",
    "phase": "execution",
    "retryable": false
  }
}
```

`version`, a nonempty string `id`, and `status` are required. Status is exactly one of `queued`, `running`, `completed`, `failed`, or `cancelled`. Map a timed-out or interrupted job to `failed`; preserve the specific reason in its stable error code and phase. A later failure reading the results of a completed job is a tool-call failure; it does not change that job's completed state.

`error` is optional. When present, its nonempty string `code`, `kind` and `message`, and boolean `retryable`, are required. `phase` is optional and must be a nonempty string when supplied. Use the existing error kinds `not_found`, `permission_denied`, `timeout`, `invalid_args`, `transient`, `tool_failure`, or `internal`. An unknown kind degrades to `tool_failure` without guessing from the message. The explicit retryable flag is preserved. Messages are displayed to the person and must be suitable for that audience. BioRouter applies its credential redaction to this metadata before it reaches the UI or a script's telemetry.

The payload is deliberately strict: malformed fields, unknown status/version, and extra envelope/error fields are ignored rather than interpreted as a failure. Adding new fields requires a new contract version. Servers without the envelope keep their previous behavior. Ordinary content, structured result JSON, and prose containing `status: failed` are never scanned for operation semantics.

## Consumers

The transcript preserves the envelope across save and reload. The tool card shows **Operation failed** separately from **Status lookup completed successfully**, and keeps the status payload available. Queued, running, completed and cancelled operations are not failures.

The failure census reports failed tool calls and failed operations separately. It counts a failed operation once per extension and operation id, even if several status or wait tools report it repeatedly. The loop monitor can use the typed failure as no-progress evidence without changing the lookup's protocol result or reclassifying its error text.

Code Execution records an operation envelope on the corresponding `biorouter/tool-calls` telemetry record. A script can inspect or handle a failed status lookup as an ordinary returned value. BioRouter does not invent an exception or mark the outer script failed merely because it observed a failed operation. The census and nested UI still show the operation failure and its original extension. Only a result from the built-in `code_execution__execute_code` dispatch (or its resolved bare `execute_code` name) can supply this nested telemetry; another extension cannot attribute a failure to a different extension. The loop guard tracks the outer dispatch identity so repeated wrappers cannot bypass its stop. The outer result does not claim to be that operation when a script has transformed or handled the value.
