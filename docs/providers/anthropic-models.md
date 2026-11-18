# Anthropic and Claude Code models

> **What this is.** The default models and optional tiers offered by Biorouter's Anthropic API and Claude Code providers.
> **Status:** Current, verified against official documentation on 2026-11-17.
> **Audience:** end users and developers working on model providers.

The Anthropic API provider starts new selections on Claude Opus 5.5 and uses Claude Haiku 5.5 for fast work. Claude Code also starts on Claude Opus 5.5. Changing a default does not replace a model already selected for a chat.

## Choosing a model

Claude Opus 5.5, Claude Sonnet 5.5, and Claude Haiku 5.5 lead each provider's model list, in that order. Anthropic recommends Opus 5.5 for most workloads. Sonnet balances speed and capability; Haiku serves high-volume, latency-sensitive work. Claude Fable 5.2 is an optional tier for demanding reasoning and long-running agent work. [Anthropic model overview](https://platform.claude.com/docs/en/models/overview).

The four current tiers take text and image input, return text, support tools, have a one-million-token context, and support up to 128,000 output tokens. [Current model specifications](https://platform.claude.com/docs/en/models/overview). Full model identifiers are pinned snapshots, including the dateless names. Short Claude Code aliases can resolve differently with another backend or CLI version. [Model identifiers](https://platform.claude.com/docs/en/about-claude/models/model-ids-and-versions).

## Running Claude Code

| Model | Identifier | Minimum Claude Code version |
| --- | --- | --- |
| Claude Opus 5.5 | `claude-opus-5-5` | 2.1.280 |
| Claude Sonnet 5.5 | `claude-sonnet-5-5` | 2.1.284 |
| Claude Haiku 5.5 | `claude-haiku-5-5` | 2.1.293 |
| Claude Fable 5.2 | `claude-fable-5-2` | 2.1.321 |

Fable can bill separately to usage credits depending on the plan and seat. Claude Code's noninteractive mode does not show a consent prompt for those credits, so Fable remains an explicit selection. [Claude Code model configuration](https://code.claude.com/docs/en/model-config).

## Keeping an existing selection

Older served models remain available through a saved or manually entered identifier. Sonnet 4.5 is no longer advertised for new Anthropic API selections because Anthropic schedules its retirement for November 30, 2026, with Sonnet 5.5 as the replacement. Partner-operated clouds can use different retirement schedules. [Model lifecycle](https://platform.claude.com/docs/en/about-claude/model-deprecations).

## Preserving thinking history

The direct API adapter sends adaptive thinking and the binding-control beta for Opus, Sonnet and Haiku 5.5 and Fable 5.1/5.2. When Biorouter changes an earlier system prompt or history prefix, the API may drop the affected signed thinking blocks instead of rejecting the next request. Visible text and tool history remain in the request. Claude Code manages its own request history. [Fable 5.2 migration behavior](https://platform.claude.com/docs/en/models/fable-5-2/whats-new-fable-5-2).

## Related documentation

- [Coding-agent providers](coding-agents/README.md) explains local CLI setup and account use.
- [Coding-agent performance and limits](coding-agents/performance-and-limits.md) describes context and tool limits.
- [Provider selection](README.md) lists the available providers.
