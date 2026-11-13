# Older tool results and the prompt cache

> **What this is.** How Biorouter keeps a long chat's requests small and cacheable without losing anything: older, larger tool results leave the model's copy of the conversation in rare batches, a long reply is compacted between its own tool rounds (at most twice), the system prompt and the tools stay byte-identical, and the automatic context sent with each request stays where it was first sent, so each request is the previous one plus its new tail and the provider's prompt cache reaches the whole conversation.
> **Status:** Current. Code: `crates/biorouter/src/context_mgmt/elision.rs`, `context_mgmt/prompt_fingerprint.rs`, `check_if_compaction_needed`, `run_eager_compaction` and `with_saved_output_index` in `context_mgmt/mod.rs`, `crates/biorouter/src/agents/large_response_handler.rs`, `crates/biorouter/src/agents/moim.rs`, and `Agent::assemble_turn_context`, `Agent::mid_turn_compaction_due`, `Agent::run_mid_turn_compaction` and `Agent::with_saved_tool_outputs` in `crates/biorouter/src/agents/agent.rs`.
> **Audience:** people whose chats run long data analyses, and anyone changing how a request is assembled.

## Why a long chat gets expensive

Every model request sends the whole conversation again. A query result the agent read at request 40 is sent again with each of the requests after it. In the chat that motivated this work (575 requests, a median prompt of about 270k tokens), tool results were 58% of everything the model re-read, and most of them were between 2,000 and 40,000 characters: under the 10k-token cap that moves a single large result to a file, so nothing caught them.

Two things make a request cheaper. Sending less of it, and having the provider read more of it from its prompt cache, which is billed at a fraction of fresh input. A provider caches a request's prefix: from the first byte that differs from the previous request, everything is read again at the full price.

## Older tool results

Before each request, Biorouter looks at the tool results the model would be sent, newest first.

- The newest 40,000 tokens of tool output are kept as they are, and so is every result of the latest round (the answers to the model's most recent request), whatever their size.
- Older results longer than 1,500 characters can be elided. Results of the checklist tools, of skills, and of the workspace protocol tools are never elided, and neither are results with images. An error keeps its first lines.
- Nothing happens until at least 25,000 tokens of older output qualify. Then all of it goes in one batch.

An elided result is replaced by a short note in the model's copy. The note names the tool, the size of the result, the start of its first line (or an error's first lines), and the file that holds the full text. The full text is written to `<working folder>/.biorouter/tool-output/`, the same folder the large-result handler uses (a result that handler already saved for this chat keeps its existing file; a "saved at" path printed anywhere else in a result, which a web page or a document can carry, is never trusted, and a fresh copy is written instead). The note asks the model to read or search that file when it needs the data again, rather than run the tool again. If the file cannot be written, the result is not elided, and in a chat without the Developer extension (the only way to read the file back) no new batch runs at all.

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

A reply also compacts at most twice between its tool rounds, however well each compaction worked. When the trigger sits close to the fixed part of the request (the system prompt and tools, about 36k tokens on the default setup), a few tool results push a freshly compacted context straight back over it. Measured on 2026-11-10 with a 40,000-token ceiling: a model asked to show five files re-read them after every summary, and one reply compacted six times, each a 15k-token summarizer call. After the second compaction the reply goes on, or ends, without compacting again.

### The summary names the saved files

A summary cannot carry the full text of a file listing the person asked to see, so after a compaction a careful model ran the tool again. Before a compaction now, every tool result of at least 1,500 characters in the part being summarized is saved to `<working folder>/.biorouter/tool-output/` (a result an elision batch or the large-result handler already saved keeps its file), and the summary ends with a list: each tool, a short copy of its call's arguments, the result's size and the file that holds it, with a line asking the model to read or search the file instead of running the tool again. The newest 60 are listed. Results the compaction kept as they are, results of the checklist, skills and workspace protocol tools, and every result in a chat without the Developer extension (which has nothing to read the files with) are not listed. This applies to compaction at the start of a turn, between a reply's tool rounds, in overflow recovery, and in the background compaction that runs after a turn ends over the trigger.

The check measures what the model is sent, elided results included, so elision delays compaction rather than triggering it.

The context card under the composer draws a tick where compaction will really happen. On a window over 250,000 tokens that is the ceiling, not the marker's share, and the card says so.

## The saved files

Elision and compaction write each saved result, as plain text, to `<working folder>/.biorouter/tool-output/`. When the working folder cannot be written, the file goes to `biorouter_mcp_responses` in the system temp folder instead. On macOS and Linux only your account can open that folder (mode 0700) or read the files in it (mode 0600), and a folder or file an earlier version left open is tightened the next time a result is saved there. A folder that cannot be made private, such as one another account owns, is not written to: the result then stays in the conversation, and for the large-result handler only its preview survives. The `.biorouter/` folder above it keeps its permissions, because it also holds project skills, workflows and hooks that other people may share.

These files are copies of what the tools returned, query rows and file contents included, and Biorouter never removes them: they stay after the chat is deleted. Delete the `tool-output` folder to remove them. Anything that runs as your account can read them, including another chat working in the same folder with a shell, whatever either chat's privacy tier (see [Privacy tiers](../security/privacy-tiers.md)).

## A stable prefix

The system prompt and the tools come first in every request. These were found changing between requests and are fixed:

- The prompt's clock. It stated the hour, so once an hour every chat re-read its whole prefix (measured on Versa: one changed clock line took cached tokens from 26,694 to 0). It now states the date. The automatic context (below) carries the minute.
- The skills inventory. It began with a catalog generation number that went up on every rescan of the skill folders, whether or not this chat's skills changed. The number is no longer sent.

A test builds the prompt and the tools twice, with a turn's changes in between (messages stored, the checklist written, time passing), and requires them to be identical.

Biorouter logs a short hash of the system prompt and of the tools for every request at debug level, and an info line when either changed since the chat's previous request, naming which. Look for "the request's fixed prefix changed" when a chat's cache rate is low.

## The automatic context stays where it was sent

With each request the model also gets a block of automatic context: the time, the working folder, a map of the workspace, the checklist, and the other built-in capabilities' current state (an orchestration plan, the preview panel, the code-execution modules). It used to be removed and put back at the end of every request. A provider caches a request's prefix, so request n was never a prefix of request n+1: where request n had the block, request n+1 had the model's next message. Measured on Versa (Azure OpenAI Responses) on 2026-11-10, whose gateway reuses an earlier request only when that WHOLE request is a prefix of the new one: the cached figure was the same 35,572 tokens (the system prompt and tools) on every request of every chat, and the conversation itself was never read from the cache. Providers that cache any shared prefix (Anthropic's cache breakpoints, OpenAI's automatic cache, llama.cpp's reuse of its KV cache) lost less, the last tool round of each request, but lost it on every request.

Now each block stays where it was first sent, and a new block is added at the end of a request only when what it says changed, holding only the parts that changed, under a first line saying the rest still holds. So when a request only adds messages, the previous request is a byte-identical prefix of it, and a change still reaches the model at the end of the request it happened in. The blocks' places and exact text are stored with the chat (`moim_anchors.v1` in its extension data), so a restarted app sends the same bytes.

How often each part is sent again:

| Part | Sent again |
|---|---|
| The checklist, an orchestration plan, the preview panel, the code-execution modules, the working folder | As soon as it changes. A part that goes away is named as no longer current. |
| The time (to the minute) | With any other change, or on its own 15 minutes after it was last stated. |
| The workspace map | Only on the time's 15-minute schedule, because it changes whenever the agent writes a file. |
| The planning gate's "write the checklist first" | Never kept: it rides at the very end of each request it is raised on, and the next request carries no trace of it. |

Old blocks are dropped once a later block has restated everything they said: at the next elision batch, which changes the request there anyway, or once they add up to 20,000 tokens. A compaction, an edit or a rewind that removes a block's place in the history starts over with one full block. `BIOROUTER_MOIM_ANCHORED=false` restores the old block at the end of every request.

A test builds several turns of requests (tool calls, checklist changes, time passing, a restart, an elision batch and a compaction) and checks, in the Responses, Anthropic and Chat Completions wire formats, that each request is the previous one plus its new tail except at the batch and the compaction.

## Settings

| Key | Default | Meaning |
|---|---|---|
| `BIOROUTER_TOOL_RESULT_ELISION` | `true` | `false` sends every tool result in full again, including results already elided. |
| `BIOROUTER_TOOL_RESULT_ELISION_PROTECT_TOKENS` | `40000` | The newest tool output, in estimated tokens, that is never elided. |
| `BIOROUTER_TOOL_RESULT_ELISION_MIN_PRUNABLE_TOKENS` | `25000` | How much older output must qualify before a batch runs. |
| `BIOROUTER_TOOL_RESULT_ELISION_MIN_CHARS` | `1500` | A result shorter than this is never elided. |
| `BIOROUTER_AUTO_COMPACT_MAX_TOKENS` | `200000` | The token count at which a chat compacts whatever its window; `0` removes it. |
| `BIOROUTER_MOIM_ANCHORED` | `true` | `false` sends the automatic context as one block at the end of every request again. |

Token figures here are estimates at 4 characters a token, the same every time, so the same chat always plans the same batch.

## Related documentation

- [Long-running tasks](long-running-tasks.md)
- [Waiting on jobs](waiting-on-jobs.md)
- [Context engineering](context-engineering.md)
- [Compaction preservation marker](compaction-preservation-marker.md)
- [Privacy tiers](../security/privacy-tiers.md)
- [Environment variables](../configuration/environment-variables.md)
