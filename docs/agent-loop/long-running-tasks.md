# Long-running tasks

> **What this is.** How a long agent task is allowed to run, and the limits that keep its size and cost bounded as it goes.
> **Status:** Current.
> **Audience:** end users

Ordinary chats continue until the task finishes, you press Stop, or a real limit or failure requires the agent to stop. There is no default cap of 100 turns or 200 tool calls. A turn here means one trip through the agent loop, which can include several tool calls.

## Choosing a limit

In **Settings > Chat > Approvals > Max turns**, choose **Unlimited**, or choose **Set a
limit** and enter a positive whole number. Clearing the field or entering an invalid
value does not change the saved setting. A positive limit already saved in an earlier
version remains in effect; it is never silently removed.

From the command line, `--max-turns 0` selects Unlimited for that run. `biorouter configure` accepts the same value. The global settings are:

| Setting | Default | Meaning |
| --- | --- | --- |
| `BIOROUTER_MAX_TURNS` | `0` | Unlimited agent-loop turns; a positive number sets a turn limit |
| `BIOROUTER_MAX_TOOL_CALLS` | `0` | Unlimited tool calls; a positive number sets a tool-call limit |

A per-run setting takes precedence over the global setting. When no per-run setting is supplied, the global value applies. Reasoning effort adjusts positive limits and leaves Unlimited unchanged.

## When work gets stuck

Unlimited does not disable loop recovery. Repeated failures and repeated actions without changing results can prompt the agent to change its approach. A persistent loop stops with an explanation rather than running forever. Repeated successful reads, new pages of results, and waiting on work that is still running must not be mistaken for a failed loop.

Stop remains available throughout the task. Explicit time, token and cost budgets still apply when configured. Usage limits for a chat, for every chat by default, and for the month stop a chat before its next model request; see [Usage limits](usage-limits.md). Context compaction still lets the task continue as its history grows; Unlimited does not enlarge the model's context window or guarantee that every task will finish.

Subagents, Crew task workers and built apps keep their separately configured limits.

## Keeping a long task affordable

Every request the agent sends carries the whole conversation so far. A tool result that stays in the chat is therefore paid for again by every later request, and a long task can cost far more than the size of any one step suggests. Several limits bound that growth.

### Large tool results

A tool result over 10,000 tokens (about 40,000 characters) is saved in full to a file under `.biorouter/tool-output/` in the chat's working folder. The chat keeps a head and tail preview of about 2,000 tokens and the file's path, and the agent reads or searches the file for the rest. The folder is hidden from git. The limit was 25,000 tokens until November 2026.

| Setting | Default | Meaning |
| --- | --- | --- |
| `BIOROUTER_LARGE_RESPONSE_TOKENS` | `10000` | Largest tool result kept in the chat, in tokens |
| `BIOROUTER_LARGE_RESPONSE_PREVIEW_TOKENS` | `2000` | Size of the preview kept for a saved result |

The developer tools apply their own bounds before that:

- **Shell output** over 100 lines or over 40,000 characters is saved to a temporary file, and the agent sees the last lines, at most 8,000 characters of them. Output that is short in lines but very wide, such as one long JSON line, is now caught as well.
- **Viewing a file** returns at most 40,000 characters at a time. A larger view is refused with a line range that fits, or, when one line alone is too long, with a way to read part of it.
- **Waiting on a background command** with `shell_status` blocks for at most 270 seconds per call, which keeps the wait inside the 300-second tool-call timeout. It used to advertise 600 seconds, and any wait past 300 failed as a timeout.

To wait on an external job without a model request per check, use `code_execution__wait_for` (up to 10 minutes, the turn stays open) or `code_execution__watch_job` (the turn ends and the chat is resumed once when the job finishes); see [Waiting on jobs](waiting-on-jobs.md).

### Automatic compaction

When the conversation grows past a limit, Biorouter summarizes its older part and keeps the most recent turns word for word. The limit is the lower of a share of the model's context window and a fixed number of tokens:

| Setting | Default | Meaning |
| --- | --- | --- |
| `BIOROUTER_AUTO_COMPACT_THRESHOLD` | `0.8` | Share of the context window; `0` turns automatic compaction off |
| `BIOROUTER_AUTO_COMPACT_MAX_TOKENS` | `200000` | Token count at which a chat compacts whatever the window; `0` removes this ceiling |

On a 128,000-token model the share decides (102,400 tokens). On a model with a 1,050,000-token window the share alone would wait for 840,000 tokens, so a long chat never compacted and each request re-sent hundreds of thousands of tokens. Some models also bill a whole request at a higher rate once its input passes 272,000 tokens, so the 200,000-token ceiling keeps requests under that tier as well. The limit is checked between turns and also before every request of a long reply, so a single turn with many tool calls does not grow far past it. Inside a reply, compaction summarizes the older part first, and everything once if that is not enough; it does not summarize again in the same reply. Older tool results also leave the model's copy in batches before compaction is needed; see [Older tool results and the prompt cache](stale-tool-results.md).

A compaction's own model calls are counted in the chat's usage, including a summary that was rejected and retried and a background compaction whose result was discarded because the chat changed while it ran.

### How the agent is asked to work

The system prompt asks the agent to treat query results as data to process in code rather than text to read: return counts, schemas and a few sample rows, write bulk results to a file, select only the columns it needs, and leave long free-text fields such as clinical notes out of the chat unless you asked to read them. It also asks the agent to put independent tool calls in one message, to report checklist progress in the `todo` field of its next real tool call rather than in a call of its own (the first open item counts as in progress by itself, so it reports only what is done or blocked), and to wait on running work with one blocking wait (`code_execution__wait_for`, or `code_execution__watch_job`, which ends the turn and resumes it when the job finishes) rather than checking every few seconds. See [Todo capability](../extensions/built-in/todo.md#how-progress-reaches-the-checklist).

## Related documentation

- [Context engineering](context-engineering.md)
- [Older tool results and the prompt cache](stale-tool-results.md)
- [Waiting on jobs](waiting-on-jobs.md)
- [Compaction preservation marker](compaction-preservation-marker.md)
- [Environment variables](../configuration/environment-variables.md)
- [Usage limits](usage-limits.md): per-chat, default and monthly limits in tokens or dollars.
