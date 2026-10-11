# Llama Server provider

This folder covers the bundled **Llama Server** (`llamacpp`) provider: the zero-setup
local-inference path, where the desktop app ships a pinned `llama-server` binary from llama.cpp
(build b11429) and manages it as a sidecar process. It offers exactly two models, Qwen3.8 27B
(the default) and Gemma 4 31B. The pages here say what those models are and how the sidecar
runs them, and how to confirm, after a change, that both still load, answer, call tools and
stream, and that the sidecar's security invariants still hold.

Come here when you are choosing between the two local models, changing the `llamacpp` provider
or its managed sidecar, bumping the pinned llama.cpp build, or altering `MODEL_CATALOG`. The
other local provider, [Ollama](../ollama/README.md), serves the same two models from a server you
install yourself, and Llama Server can start from a GGUF already pulled into Ollama. If you are
deciding *which* provider to use, read
[choosing a model provider](../../getting-started/choosing-a-model-provider.md). Hosted, API-key
providers are documented as flat files one level up, [z.ai (GLM)](../zai-glm.md) and
[Xiaomi MiMo](../xiaomi-mimo.md), and none of the sidecar, download or local-memory concerns here
apply to them. For the environment variables themselves, see
[environment variables](../../configuration/environment-variables.md).

## Documents

| Document | What it covers |
|----------|----------------|
| [Llama Server model catalog](model-catalog.md) | The two models and their sources, memory needs and suitability ratings, what a name from an earlier catalog does, the order the sidecar looks for a copy on disk, and the context window, request, thinking and speed policy. Current, checked against the code on 2026-10-10. |
| [Llama Server model catalog QA checklist](model-catalog-qa-checklist.md) | A repeatable checklist, plus the integration suite and survey harness that execute most of it, for verifying each catalog model's availability, tool calling at the size of a real first turn, streaming and speed, the launch flags to expect, and the sidecar's security invariants. Current, rewritten for build b11429 on 2026-10-10. |

## Related documentation

- [Ollama provider](../ollama/README.md): the other local provider, with the per-model findings
  on tool-call protocols.
- [Environment variables](../../configuration/environment-variables.md): the `LLAMACPP_*` and
  `BIOROUTER_LLAMACPP_BIN` settings.
- [Choosing a model provider](../../getting-started/choosing-a-model-provider.md): where Llama
  Server sits among the local, institutional and commercial providers.
- [Model provider integration references](../README.md): the parent index.
- [Debugging the dev GUI with agent-browser](../../desktop-ui/agent-browser-debugging.md): how to
  drive the dev app for the checklist's manual GUI section.
