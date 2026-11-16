# Content-filter refusals from Azure and OpenAI-compatible providers

> **What this is.** What Biorouter says and does when a provider's content filter blocks a
> request or a reply, and when the filter fails to run, on Azure OpenAI, UCSF Versa API Azure,
> OpenAI and the custom providers built on OpenAI's format.
> **Status:** Current (November 2026).
> **Audience:** maintainers changing these providers, and anyone reading a content-filter message
> in a chat.

Azure runs a content filter on every request and every reply, and some OpenAI-compatible servers
do too. Before November 2026 a filtered reply could read as an empty reply, and a filter that only
failed to run could end the turn. The handling below lives in
`crates/biorouter/src/providers/formats/openai.rs` and is shared by `azure.rs`, `versa_azure.rs`,
`openai.rs` and the Responses route (`formats/openai_responses.rs`).

## A blocked request

When the filter blocks the prompt, the provider answers HTTP 400 with `error.code`
`content_filter`. Biorouter reports it in its own words and names the categories the filter marked
as filtered, for example:

```text
Request failed: Azure's content filter blocked this request (flagged: jailbreak, violence). Revise the request or contact your provider administrator.
```

The request is sent once. The message is classified as a policy refusal, so it is not retried: the
same request is filtered the same way. The provider's own error text is never shown, because it is
provider prose that could echo the prompt. When the error carries no Azure verdict, the sentence
says "The provider's content filter" and names no categories (`content_filter_refusal`).

## A blocked reply

When the filter blocks the reply, Azure answers with `finish_reason: "content_filter"` and the
flagged categories. Biorouter reads that before it reads the message, so the chat shows the block
rather than an empty reply:

```text
Request failed: Azure's content filter blocked this reply (flagged: self harm). Revise the request or contact your provider administrator.
```

This holds for streamed and non-streamed replies (`content_filter_block_in_completion`). On the
Responses route the response names neither whose filter fired nor what it flagged
(`incomplete_details.reason` of `content_filter`), so the sentence reads "The provider's content
filter blocked this reply." with no categories (`content_filter_stopped_reply`).

## A filter that did not run

Azure sometimes marks a reply with a filter-failure annotation, `content_filter_error` with the
message "The contents are not filtered". That is not a block. The reply is delivered whole and the
turn goes on. The daemon log gets one warning per reply, naming the code and, only when it is
Azure's own sentence, the message; never the prompt or the reply.

## What the log records

Each block logs one warning, "Provider content filter blocked the reply" or "Provider content filter
refused the request", with the code and the flagged category names only. Nothing from the prompt or
the provider's message is logged.

## Related documentation

- [Choosing a model provider](../getting-started/choosing-a-model-provider.md): the provider list, including Versa API Azure and Versa API Bedrock, their default models and prices.
- [Common problems and fixes](../troubleshooting/common-problems-and-fixes.md): what to do about other provider errors.
- [Model provider integration references](README.md): the index of this folder.
