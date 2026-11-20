# Prompt caching on Amazon Bedrock and Versa Bedrock

> **What this is.** How the `aws_bedrock` and `versa_bedrock` providers mark the Converse API's
> prompt-caching checkpoints, which models get them, how a gateway that refuses them is handled,
> and how to turn them off.
> **Status:** Current, written 2026-11-19 for issue #527 (Biorouter 1.94.6). Model list checked
> against the AWS prompt-caching guide on 2026-11-19.
> **Audience:** maintainers of the Bedrock providers, and anyone reading Bedrock usage or cost.

Before 1.94.6 neither Bedrock provider sent a single `cachePoint`, so the system prompt, the tool
list and the whole conversation were billed as fresh input on every model call. One agentic session
on `versa_bedrock` with Claude Opus 5.5 averaged about 96,000 input tokens per call with no cache
reads. The Anthropic provider already cached all three. The request builder is
`bedrock_request_parts` in `crates/biorouter/src/providers/formats/bedrock.rs`, shared by both
providers and by both APIs (Converse and ConverseStream), so the two cannot disagree.

## Where the checkpoints go

AWS reads checkpoints in the order tools, system, messages, and Claude accepts at most four in one
request. Biorouter places:

1. one after the last tool spec, when the request offers tools;
2. one after the system prompt, when it is not empty;
3. one at the end of each of the two most recent user turns that carry content (text, a tool
   result, an image or a document). An empty user turn is skipped.

That is four at most. Nothing else in the request changes: only `cachePoint` blocks are added, and
the stored conversation is never touched. A checkpoint before the model's minimum prefix length is
accepted by AWS and simply not cached, so Biorouter does not count tokens first. Usage already keeps
cache reads and cache writes apart (`from_bedrock_usage`), so the gauge and the cost estimate see
them as soon as the checkpoints are sent.

## Which models get them

Exactly the Claude models in the "Supported models, Regions, and explicit caching limits" table of
the [AWS prompt-caching guide](https://docs.aws.amazon.com/bedrock/latest/userguide/prompt-caching.html)
(`BEDROCK_PROMPT_CACHING_MODELS`), with or without an inference-profile prefix such as `us.` or
`global.`. Every model in Biorouter's Bedrock list is in that table except Claude Sonnet 4.5, which
AWS does not list, so it gets none. A model the table does not name, an application inference
profile ARN, and Amazon Nova (which takes no checkpoint in `tools`) get none either: AWS answers an
unsupported checkpoint with a `ValidationException`, not a quiet no-op.

## A gateway that refuses them

Versa Bedrock is a proxy in front of Bedrock, and `aws_bedrock` can be pointed at one. A gateway
that does not pass `cachePoint` through answers HTTP 400. When the request carried checkpoints and
the failure is that refusal and nothing else, Biorouter sends the same request once more without
them, and the provider instance remembers the model: neither API sends it checkpoints again.

The test for "that refusal and nothing else" is strict, because the answer is a second request
(`bedrock_cache_point_rejected`): an HTTP 400 typed as a `ValidationException` or not typed at all,
every error code in the raw response agreeing with that, and a message that says the checkpoints
are what was refused. A 401 or 403, any other code, a conflicting code, an unrelated validation
error, and a body that is not a small JSON object keep their ordinary handling and send nothing
more.

A refusal can also arrive after the stream has opened, as a stream event. Then the model is
remembered the same way and the error is reported as a retryable server fault, so the agent's one
retry sends the request again without checkpoints.

## Turning them off

Set `BEDROCK_PROMPT_CACHING` to `false` (in `config.yaml` or the environment). It is shared by both
Bedrock providers, like the other `BEDROCK_*` tuning keys, and is read when the provider is built.
A value that is not `true` or `false` turns caching off and logs a warning. The key changes nothing
about where requests go, so it is classified as a non-destination key in
`providers/destination_keys.rs`.

## Verifying

```bash
cargo test -p biorouter --lib -- providers::bedrock providers::versa_bedrock providers::formats::bedrock::prompt_cache_tests
```

The wire tests capture the real SDK request and assert the four checkpoints, the strict refusal
test, the single resend and that the refusal is remembered per instance and per model. A live
check: two calls in one session on a listed model should show `cacheReadInputTokens` above zero on
the second.

## Related documentation

- [Environment variables](../configuration/environment-variables.md): `BEDROCK_PROMPT_CACHING` beside the other Bedrock settings.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md): the Bedrock and Versa Bedrock model lists.
- [Model provider integration references](README.md): the index of this folder.
