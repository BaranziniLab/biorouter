# Long-running tasks

Ordinary chats continue until the task finishes, you press Stop, or a real limit or failure requires the agent to stop. There is no default cap of 100 turns or 200 tool calls. A turn here means one trip through the agent loop, which can include several tool calls.

## Choosing a limit

In **Settings > Chat > Approvals > Max turns**, choose **Unlimited** or enter a positive number. Zero means Unlimited. Clearing the field does not change the saved setting. A positive limit already saved in an earlier version remains in effect; it is never silently removed.

From the command line, `--max-turns 0` selects Unlimited for that run. `biorouter configure` accepts the same value. The global settings are:

| Setting | Default | Meaning |
| --- | --- | --- |
| `BIOROUTER_MAX_TURNS` | `0` | Unlimited agent-loop turns; a positive number sets a turn limit |
| `BIOROUTER_MAX_TOOL_CALLS` | `0` | Unlimited tool calls; a positive number sets a tool-call limit |

A per-run setting takes precedence over the global setting. When no per-run setting is supplied, the global value applies. Reasoning effort adjusts positive limits and leaves Unlimited unchanged.

## When work gets stuck

Unlimited does not disable loop recovery. Repeated failures and repeated actions without changing results can prompt the agent to change its approach. A persistent loop stops with an explanation rather than running forever. Repeated successful reads, new pages of results, and waiting on work that is still running must not be mistaken for a failed loop.

Stop remains available throughout the task. Explicit time, token and cost budgets still apply when configured. Context compaction still lets the task continue as its history grows; Unlimited does not enlarge the model's context window or guarantee that every task will finish.

Subagents, Crew task workers and built apps keep their separately configured limits.
