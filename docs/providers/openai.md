# OpenAI models

> **What this is.** The default models and request capabilities of Biorouter's OpenAI API provider.
> **Status:** Current. Model identifiers checked against official documentation on 2026-11-17.
> **Audience:** users configuring OpenAI and developers maintaining its provider.

New OpenAI configurations start with GPT-6.2 Astra, the starting flagship recommended in
[OpenAI's model catalog](https://developers.openai.com/api/docs/models). An explicit model selection
stays selected when Biorouter's bundled catalog changes.

## Choosing a model

| Model | API identifier | Typical use |
| --- | --- | --- |
| GPT-6.2 Astra | `gpt-6.2-astra` | Demanding reasoning and coding |
| GPT-6.2 Sol | `gpt-6.2-sol` | Everyday work with lower cost |
| GPT-6.1 Terra | `gpt-6.1-terra` | Writing, visual reasoning and computer use |
| GPT-6 Luna | `gpt-6-luna` | Focused tasks and background titles or summaries |

GPT-6.1 Sol and the older supported models remain available as explicit choices. Availability at an
OpenAI-compatible gateway depends on that gateway's own model catalog.

## Requests and limits

The latest models accept text and images and produce text. Their API context window is 1,050,000
tokens, including up to 128,000 output tokens. Biorouter sends tool-bearing GPT-6 requests through
the Responses API and omits unsupported sampling parameters. OpenAI's
[migration guide](https://developers.openai.com/api/docs/guides/latest-model#migration-quickstart)
describes the required reasoning and conversation-history behavior.

The Codex coding-agent provider uses the account's client catalog and working context. Its model
availability, subscription accounting and context limits are separate from the OpenAI API provider.

## Related documentation

- [Provider documentation](README.md) covers other providers and deployment-specific setup.
- [Coding-agent performance and limits](coding-agents/performance-and-limits.md) explains Codex
  discovery and reasoning selection.
- [OpenAI model specifications](https://developers.openai.com/api/docs/models) provide current
  availability, prices and lifecycle information.
