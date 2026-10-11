# Ollama provider

This folder documents the **Ollama** provider (`ollama`): local models served by an
[Ollama](https://ollama.com) server the user installs and runs, by default on this machine at
`http://localhost:11434`. Biorouter talks to it over Ollama's native `POST /api/chat`, sends the
context window it wants on every native request, and rejects suspiciously short reported
prompt usage. This check depends on the token counts the server provides. The same code also
serves any custom provider declared with the `ollama` engine in a
`custom_providers` file. The pages here cover how Biorouter talks to Ollama, which settings
change that, which local models work as an agent, and what Biorouter does with the many ways
a model can return a tool call.

Come here when you are **choosing a local model for Ollama, configuring the provider, or
changing `crates/biorouter/src/providers/ollama.rs` or `formats/ollama.rs`**. If you want the
zero-setup local path instead, the bundled **Llama Server** (`llamacpp`) runs the two models
Biorouter recommends for Ollama without a separate install; it is documented in
[`../llama-server/`](../llama-server/README.md), and it can start from a GGUF model already
pulled into Ollama. If you are deciding between local, institutional and commercial
providers, start at [choosing a model provider](../../getting-started/choosing-a-model-provider.md).

## Documents

| Document | What it covers |
| --- | --- |
| [Context window and settings](context-window-and-settings.md) | How a request to Ollama is built: the native API and its restricted `/v1` compatibility fallback, how the context window (`num_ctx`) is chosen, the truncation defence, thinking, keep-alive and timeouts, every `OLLAMA_*` setting, the two recommended models, and what to do when a model "returns JSON instead of doing the task". Current, measured on Ollama 0.40.2 in October 2026. |
| [Tool-call protocols](tool-call-protocols.md) | How Ollama turns a model's output into a tool call (five paths, chosen per model), the per-model results of probing 12 models on the wire, which local models work well as an agent and which to avoid, the formats a model writes when a call leaks into text, and the guards on recovering such a call. Current, measured 2026-10-10. |

## Related documentation

- [Llama Server provider](../llama-server/README.md): the bundled llama.cpp sidecar, which
  offers those two models in its managed catalog and reuses Ollama's model store.
- [Choosing a model provider](../../getting-started/choosing-a-model-provider.md): where Ollama
  sits among the local, institutional and commercial providers.
- [Environment variables](../../configuration/environment-variables.md): the `OLLAMA_*`
  settings among every other configuration variable.
- [Model provider integration references](../README.md): the parent index.
