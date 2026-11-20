# Stream faults, empty replies and streamed reasoning

> **What this is.** How the OpenAI-compatible stream decoder classifies an error that arrives inside
> a stream, what the agent does with an empty reply, and how OpenRouter's streamed
> `reasoning_details` are stored and replayed.
> **Status:** Current, written 2026-11-19 for issues #542 and #548 (Biorouter 1.94.6).
> **Audience:** maintainers of the OpenAI-compatible providers and the agent loop.

A review of BioRouterBench 1.1.0 found two upstream faults that ended a Biorouter turn where Codex
and Claude Code carried on: an error frame inside an OpenRouter stream ended the turn with
"Retrying will not help", and an empty reply ended the run as a normal finish with exit code 0 and
the answer missing. The same review found that a streamed reply stored one `reasoning_details`
entry per streamed delta, so replay resent tens of thousands of entries. The decoder lives in
`crates/biorouter/src/providers/formats/openai.rs` and serves OpenRouter, OpenAI, Azure, Databricks,
xAI, z.ai, the local servers and the custom OpenAI-format providers.

## An error inside a stream

A chunk carrying an `error` object is classified from its structured fields only: the numeric
`code`, the `type` and OpenRouter's `metadata.error_type`. The provider's message is never shown and
never decides a retry, because it can echo the prompt. In this order:

| The frame says | Result |
| --- | --- |
| llama-server's `exceed_context_size_error` | Context length, with the two numbers it carries |
| code 401 or 403, or `authentication`, `permission_denied` and similar | Authentication: not retried, credentials refreshed on the next turn |
| code 402, or `payment_required`, `insufficient_quota` | Quota: not retried |
| `content_filter`, `content_policy_violation`, `refusal` | The provider's content filter blocked the reply: not retried |
| `context_length_exceeded` | Context length: compaction runs |
| any other `type` (such as `invalid_request_error`), or another 4xx code | The withheld "Provider reported an error while streaming" error: not retried |
| code 500 with type `server_error` | llama-server's server fault, unchanged (its "Compute error." and shared-cache sentences keep their meaning) |
| code 408, 429 or 5xx, or a transient `error_type` (`server`, `timeout`, `provider_overloaded`, `provider_unavailable`, `rate_limit_exceeded`) | A temporary failure, retried once by the agent, named by its code |

A choice with `finish_reason: "error"` beside an error object we do not otherwise recognise is a
temporary failure too, provided the code is a number or absent and no `error_type` is given:
OpenRouter's frames in the benchmark carried no string code and no type. A word we do not know as
the code keeps the frame fatal. An error finish with no error object at all used to decode as a
normal stop; it is now a temporary failure.

The agent retries a temporary failure once (`BIOROUTER_PROVIDER_ERROR_RETRIES`, default 1). If text
had already streamed, it stays in the conversation and the model is told to continue rather than
start over; tool calls that already ran are never run again. A mid-stream 429 is retried at once,
and a second 429 then arrives as an HTTP status, where the provider's own backoff handles it.

## An empty reply

A request that completes with no text, no thinking, no tool call and no record of work a coding
agent already did is an empty reply. A reply of whitespace counts. Nothing reached the person and
nothing ran, so the agent asks once more, with a note only the model sees ("Your previous reply was
empty…") and a passing notice for the person. The budget belongs to the whole reply and is never
reset, so a tool call between two empty replies does not buy a second attempt. A second empty
reply ends the turn with a stored notice and a `provider_failure` abort, which the CLI exits with as
a provider failure (exit code 70) rather than 0. A refusal, a filtered reply and a length cut are
not empty replies: a retry would only repeat them.

A reply made of OpenRouter's structured reasoning and nothing else is not empty. The decoder turns
its readable text into one unsigned thinking block, which the OpenAI-format request builder drops
on replay; reasoning made only of encrypted entries adds nothing.

## Streamed reasoning details

OpenRouter streams one reasoning block as many deltas. The decoder now joins adjacent fragments of
one block as they arrive, at both places it reads them (before a tool call starts and while its
arguments stream), so a reply stores what a non-streamed reply would. The rule is narrow, because a
wrong merge breaks replay for that model:

- only `reasoning.text` (its `text`) and `reasoning.summary` (its `summary`) merge, and only into
  the entry just before with the same `type`, `id`, `index` and `format`; an absent field matches
  only an absent one;
- an entry with a `signature` is closed; a signature that arrives alone on a block's last fragment
  is kept on that block;
- `reasoning.encrypted` never merges, so its `data` is replayed byte for byte and in order;
- a field the rule does not know, or content that is not a string, keeps the delta as its own
  entry.

Replay is unchanged: a tool request stamped with OpenRouter's provenance carries the array, and the
next request inserts it into the matching assistant message once.

## Verifying

```bash
cargo test -p biorouter --test openrouter_stream_faults --test openrouter_reasoning_fragments --test output_recovery_agent_loop
cargo test -p biorouter --lib -- providers::formats::openai providers::openrouter
```

## Related documentation

- [Content-filter refusals](content-filter-refusals.md): the policy refusal an in-stream `content_filter` code becomes.
- [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md): what a person sees when a provider fails mid-reply.
- [Model provider integration references](README.md): the index of this folder.
