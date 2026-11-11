# Older tool results and the prompt cache

> **What this is.** How Biorouter keeps a long chat's requests small without losing anything: older, larger tool results leave the model's copy of the conversation in rare batches, a long reply is compacted between its own tool rounds, and the part of every request that comes before the conversation (the system prompt and the tools) stays byte-identical so the provider's prompt cache keeps working.
> **Status:** Current. Code: `crates/biorouter/src/context_mgmt/elision.rs`, `context_mgmt/prompt_fingerprint.rs`, `check_if_compaction_needed` in `context_mgmt/mod.rs`, and `Agent::assemble_turn_context`, `Agent::mid_turn_compaction_due` and `Agent::run_mid_turn_compaction` in `crates/biorouter/src/agents/agent.rs`.
> **Audience:** people whose chats run long data analyses, and anyone changing how a request is assembled.

## Why a long chat gets expensive

Every model request sends the whole conversation again. A query result the agent read at request 40 is sent again with each of the requests after it. In the chat that motivated this work (575 requests, a median prompt of about 270k tokens), tool results were 58% of everything the model re-read, and most of them were between 2,000 and 40,000 characters: under the 10k-token cap that moves a single large result to a file, so nothing caught them.

Two things make a request cheaper. Sending less of it, and having the provider read more of it from its prompt cache, which is billed at a fraction of fresh input. A provider caches a request's prefix: from the first byte that differs from the previous request, everything is read again at the full price.

## Older tool results

Before each request, Biorouter looks at the tool results the model would be sent, newest first.

- The newest 40,000 tokens of tool output are kept as they are, and so is every result of the latest round (the answers to the model's most recent request), whatever their size.
- Older results longer than 1,500 characters can be elided. Results of the checklist tools, of skills, and of the workspace protocol tools are never elided, and neither are results with images. An error keeps its first lines.
- Nothing happens until at least 25,000 tokens of older output qualify. Then all of it goes in one batch.

An elided result is replaced by a short note in the model's copy. The note names the tool, the size of the result, the start of its first line (or an error's first lines), and the file that holds the full text. The full text is written to `<working folder>/.biorouter/tool-output/`, the same folder the large-result handler uses (a result that handler already saved keeps its existing file). The note asks the model to read or search that file when it needs the data again, rather than run the tool again. If the file cannot be written, the result is not elided, and in a chat without the Developer extension (the only way to read the file back) no new batch runs at all.

What stays the same:

- The transcript you see. Only the copy sent to the model changes. The stored chat, the desktop, the CLI, exports and compaction's summarizer all keep every result in full.
- The tool calls. The note keeps the result's place and id, and the model still sees the arguments of the call it made.

### Why batches, and why they are stored

A note that changed every request, or a rule that elided one more result each turn, would move the prefix every request and lose the cache each time. Simulated on the motivating chat, such a sliding rule broke the cache on 278 requests and made cache-priced spend 7.6% higher than doing nothing. So the set of elided results is stored with the chat (`tool_elision.v1` in its extension data) and only grows in batches, and a note is a fixed function of what is stored. Between two batches, each request is the previous one plus the new tail, byte for byte. A restarted app or a resumed chat sends the same notes.

### What it saves

Simulated on the motivating chat's sizes (no content was read; 3.5 characters a token; cached input at 0.1 times the fresh price):

| Policy | Input re-read | Cost with a warm cache | Cost with a cold cache |
|---|---|---|---|
| Elision alone | down 36.9% | down 32.9% | down 36.8% |
| Compaction at 200k tokens alone | down 64.7% | down 59.8% | down 64.3% |
| Both | down 63.0% | down 57.4% | down 62.7% |

The 200k compaction ceiling removes most of the same bytes, so with the ceiling in place elision adds no further token saving on this chat. What it adds is that nothing is lost: an elided result is one file read away, while a summary is not, and the chat needed two compactions instead of three. It saves the most where compaction fires late or never, such as a chat on a large window with the ceiling turned off.

## Compacting a long reply

Auto-compaction fires at the lower of the threshold share of the window (`BIOROUTER_AUTO_COMPACT_THRESHOLD`, 80%) and the token ceiling (`BIOROUTER_AUTO_COMPACT_MAX_TOKENS`, 200,000). It used to be checked only at the start of a turn and after it, so a single reply with hundreds of tool calls could grow far past the ceiling with nothing checking. The check now also runs before every request of a reply after its first. When it fires, the chat is compacted there, with the same notice, the same hooks and the same race-safe save as a compaction at the start of a turn.

Compaction keeps the last few turns as they are, and a long reply is one of them, so summarizing the older part may not get the context under the trigger. When the next check finds it still over, the whole history is summarized once. If it is still over after that, the reply is not compacted again; a request the provider refuses for its size still reaches the overflow recovery it always had.

The check measures what the model is sent, elided results included, so elision delays compaction rather than triggering it.

The context card under the composer draws a tick where compaction will really happen. On a window over 250,000 tokens that is the ceiling, not the marker's share, and the card says so.

## A stable prefix

The system prompt and the tools come first in every request. These were found changing between requests and are fixed:

- The prompt's clock. It stated the hour, so once an hour every chat re-read its whole prefix (measured on Versa: one changed clock line took cached tokens from 26,694 to 0). It now states the date. The message the agent gets with each request still carries the minute.
- The skills inventory. It began with a catalog generation number that went up on every rescan of the skill folders, whether or not this chat's skills changed. The number is no longer sent.

A test builds the prompt and the tools twice, with a turn's changes in between (messages stored, the checklist written, time passing), and requires them to be identical.

Biorouter logs a short hash of the system prompt and of the tools for every request at debug level, and an info line when either changed since the chat's previous request, naming which. Look for "the request's fixed prefix changed" when a chat's cache rate is low.

## Settings

| Key | Default | Meaning |
|---|---|---|
| `BIOROUTER_TOOL_RESULT_ELISION` | `true` | `false` sends every tool result in full again, including results already elided. |
| `BIOROUTER_TOOL_RESULT_ELISION_PROTECT_TOKENS` | `40000` | The newest tool output, in estimated tokens, that is never elided. |
| `BIOROUTER_TOOL_RESULT_ELISION_MIN_PRUNABLE_TOKENS` | `25000` | How much older output must qualify before a batch runs. |
| `BIOROUTER_TOOL_RESULT_ELISION_MIN_CHARS` | `1500` | A result shorter than this is never elided. |
| `BIOROUTER_AUTO_COMPACT_MAX_TOKENS` | `200000` | The token count at which a chat compacts whatever its window; `0` removes it. |

Token figures here are estimates at 4 characters a token, the same every time, so the same chat always plans the same batch.

## Related documentation

- [Long-running tasks](long-running-tasks.md)
- [Waiting on jobs](waiting-on-jobs.md)
- [Context engineering](context-engineering.md)
- [Compaction preservation marker](compaction-preservation-marker.md)
- [Environment variables](../configuration/environment-variables.md)
