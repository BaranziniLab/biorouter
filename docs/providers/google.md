# Google Gemini provider

> **What this is.** The model selection and request-format reference for the direct Google Gemini API provider.
> **Status:** Current, verified against Google's documentation on 2026-11-17.
> **Audience:** Developers maintaining the Google provider.

The `google` provider uses Google's GenerateContent API with `GOOGLE_API_KEY`.
`GOOGLE_HOST` overrides the API host. The provider source owns its advertised models
and defaults; saved model selections remain explicit choices.

## Model selection

Gemini 4 Hydrogen is the default. Google recommends it for everyday coding,
tool-using agents and multimodal work. Gemini 4 Argon and Carbon follow it in the
picker for deeper reasoning. Gemini 4 Neon is offered after them and is never the
default; Google lists it at $3 input and $15 output per million tokens, with implicit
caching only (pricing page read 2026-11-19). Gemini 3.5 Flash-Lite remains the
inexpensive fast model. See Google's [model selection guide](https://aistudio.google.com/docs/latest-model).

All four Gemini 4 models accept images and share 1,048,576 tokens across input,
thinking and output. Hydrogen defaults to high thinking; Argon and Carbon default
to medium. The adapter leaves those defaults in place. Google ignores sampling
parameters on this family, so the request omits temperature. Long decode requires
a continuation or background workflow; the current provider makes ordinary
GenerateContent requests. See the [Hydrogen migration guide](https://aistudio.google.com/docs/whats-new-gemini-4-hydrogen).

## Tool calls and conversation history

Gemini 4 signs individual response parts. The formatter preserves each native
part's signature across prior turns, keeps unsigned parallel calls unsigned,
and returns tool results with the original function name and call ID. Signed
visible text remains text, with its wire metadata surviving session persistence.
Unsigned visible text also retains its native origin so optional thoughts survive
later turns. Literal escapes in signed text and tool arguments remain unchanged.
The agent preserves the complete model turn before tool results, including text
after tool calls, with batching enabled or disabled. Earlier Gemini families retain
their existing signature handling.

Native thoughts stay in the saved history for Google and Vertex Gemini 4 replay.
Foreign request formatters and legacy Google omit those thoughts when a chat
changes provider or model family, while keeping the visible answer.

The relevant source is [the provider](../../crates/biorouter/src/providers/google.rs)
and [the formatter](../../crates/biorouter/src/providers/formats/google.rs).
[The regression suite](../../crates/biorouter/tests/google_gemini4_compatibility.rs)
checks catalogue order, native signatures, tool identities, persistence and streaming
without making API calls. [The agent loopback suite](../../crates/biorouter/tests/google_gemini4_agent_loop.rs)
drives the real provider and agent against a synthetic local HTTP server, including
real isolated tools and a later user turn.
It also checks provider changes, compatible Vertex replay and durable history.

## Verification

Use isolated test roots and the repository's credential-store test sandbox:

```bash
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --test google_gemini4_compatibility
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --test google_gemini4_agent_loop
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --lib providers::formats::google::tests
BIOROUTER_DISABLE_KEYRING=true cargo test -p biorouter --lib google_native_foreign_
```

These tests verify payloads and decoding. They do not establish account access or
claim that a live Gemini 4 request succeeded.

## Related documentation

- [Provider integration references](README.md): the provider documentation index.
- [Choosing a model provider](../getting-started/choosing-a-model-provider.md): provider selection and setup.
- [Environment variables](../configuration/environment-variables.md): API credentials and host overrides.
