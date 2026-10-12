# Llama Server model catalog

> **What this is.** The two models the bundled Llama Server offers, where each comes from, how
> the sidecar starts one, and the context, request and thinking policy it applies.
> **Status:** Current. Checked against `MODEL_CATALOG` and `RETIRED_MODELS` in
> [`crates/biorouter/src/providers/llamacpp.rs`](../../../crates/biorouter/src/providers/llamacpp.rs)
> and against [`llamacpp_sidecar.rs`](../../../crates/biorouter/src/providers/llamacpp_sidecar.rs)
> on 2026-10-10, at the pinned llama.cpp build b11429. The code is the source of truth.
> **Audience:** end users choosing a local model, and developers changing the catalog.

Until October 2026 the catalog offered seven Gemma 4 and Qwen3.6 models, defaulting to small
ones (Gemma 4 E4B, or Gemma 4 12B on large machines). An investigation into local models that
"kept returning JSON instead of doing the task" reproduced Ollama silently cutting the prompt
to about 2,000 tokens. Historical probes with Biorouter's whole ~31,000-40,000-token prompt
recorded 18 of 18 structured calls through Ollama for Qwen3.8 27B and Gemma 4 31B, and 9 of 9
through llama.cpp for Qwen3.8. Those probes check tool-call transport, not semantic grounding
or task-scope compliance. The catalog was cut to those two; the
[validation report](../../history/local-inference-2026-10/local-inference-validation.md) records the broader task results.

## The two models

| Name | Model | Source | Download | Memory |
| --- | --- | --- | --- | --- |
| `qwen3.8-27b` | Qwen3.8 27B, the default on every machine | `ggml-org/Qwen3.8-27B-GGUF:Q4_K_M`, the llama.cpp project's own conversion of the official weights (Qwen publishes no GGUF), with the official chat template | 19 GB | 32 GiB minimum, 48 GiB recommended |
| `gemma4-31b` | Gemma 4 31B | `google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0`, Google's own quantization-aware-trained build | 18 GB | 32 GiB minimum, 48 GiB recommended |

Both are dense models with a 262,144-token native window, and both are Apache 2.0. Each catalog
entry pins the exact GGUF file name, byte size and SHA-256 (measured 2026-10-09), because
llama.cpp's downloader cannot pin a repository revision and ggml-org has re-uploaded files under
the same names. A cached file whose size differs from the pin is still used, with a warning in
the log.

Any other GGUF can still be run by giving a raw Hugging Face spec, `owner/repo:QUANT`, as the
model name. Such a spec is validated strictly before it reaches the server.

## Memory and suitability

`/llamacpp/status` rates each model for this machine, by GPU-addressable memory: unified memory
on Apple Silicon, detected VRAM on a discrete GPU, and system RAM where the bundled build has no
GPU.

| Status | When |
| --- | --- |
| `suitable` | At least the 48 GiB recommendation. |
| `above_recommendation` | Between the 32 GiB minimum and the recommendation: it runs, with less room for other apps. |
| `below_minimum` | Under 32 GiB. The model is still offered, with a warning, because there is no smaller model to offer instead. |
| `cpu_only` | An Intel Mac (the macOS x64 build has no Metal) or the bundled Linux build (CPU only). Expect a few tokens per second and many minutes before each chat's first reply. The message points to what is faster there: a cloud or institutional provider on an Intel Mac, whose Ollama build is CPU-only too, and an Ollama build with GPU support or a cloud provider on Linux. |
| `unknown_resources` | GPU memory could not be detected. |

## Names from earlier catalogs

Configurations, saved chats, workflows and app manifests may still name a retired model:
`gemma4`, `gemma4-e2b`, `gemma4-12b`, `gemma4-26b`, `qwen3.6`, `qwen3.6-27b`, or the first
catalog's `qwen3.5-*` and `gemma-4-*` names. Such a name means two different things:

- **To chat**, it runs the default model, `qwen3.8-27b`, and the log says so once. The
  `/llamacpp/ensure` and `/llamacpp/warmup` routes and `biorouter models local pull` resolve it
  the same way, so pulling a retired name downloads the default model and says so.
- **To delete**, it keeps its own former files. `/llamacpp/delete` and
  `biorouter models local rm gemma4` free the old Gemma 4 E4B download; they never touch
  Qwen3.8's.

On a server you run yourself (`LLAMACPP_EXTERNAL_HOST`) a model name is passed through
unchanged, because that server may route on it.

## How the sidecar starts a model

The sidecar looks for a copy already on disk before downloading anything:

1. **The catalog's Hugging Face file**, if it is in the cache. It is started with `-m`, so the
   start needs no network.
2. **A GGUF pulled into Ollama**: the model layer of `qwen3.8:27b-q4_K_M` or
   `gemma4:31b-it-q4_K_M`, or of the bare tag on a machine that is not Apple Silicon. On Apple
   Silicon the bare tag pulls an MLX build, which llama-server cannot load, so it does not count
   as downloaded.
3. **A download** with llama.cpp's `-hf`, into the shared model store (`OLLAMA_MODELS`, or
   `~/.ollama/models`).

`biorouter models local list` labels each model with the copy the sidecar would start from, in
this order.

## Context window

The window is chosen by memory, not set to the model's full 262k, because a cache that size is
slow or impossible to allocate on a laptop:

| GPU-addressable memory | Window |
| --- | --- |
| 48 GiB or more | 131,072 tokens (128k) |
| Less than 48 GiB | 65,536 tokens (64k) |

There is no smaller automatic window. Biorouter's instructions and tool schemas alone take about
31,000 tokens with the default extensions and up to about 40,000 with more, so a 32k window
(the old small tier) failed the first request of every chat. If the server fails to start
because of memory at an automatic 128k, Biorouter retries startup at 64k. If startup still
fails at 64k, it stops with a message saying there is not enough memory. A memory or GPU
compute failure during generation discards the failed backend and reports guidance; the next
request starts a fresh server with the configured window. `LLAMACPP_CONTEXT_SIZE` pins a window
(`0` means automatic). The window the managed server actually allocated is read back from it,
so the context gauge matches it.

## Requests

- **One at a time.** The server runs with one request slot (`--parallel 1`;
  `LLAMACPP_PARALLEL` changes it). With llama-server's default of four slots sharing one cache,
  two requests of Biorouter's size could not both fit and both failed. A second request now
  waits inside Biorouter until the first finishes generating, and a model switch waits for
  requests in flight.
- **Progress during the prompt.** Streamed requests ask the server for `prompt_progress`
  updates, so a long first prompt keeps the connection alive.
- **Timeouts.** `LLAMACPP_TIMEOUT` limits silence, not the whole reply: 600 seconds by default,
  3,600 on a CPU-only machine. A timeout is never retried.
- **Output budget and sampling.** A request without a budget gets 16,384 tokens (32,768 with
  thinking on), and each model gets its vendor's recommended sampling unless the request sets
  its own.
- **Images.** The server starts without a vision projector, so an image in the conversation is
  replaced by a note saying the model cannot see it.

## Thinking

The server starts with thinking off (`LLAMACPP_ENABLE_THINKING=true` turns it on), and every
request from Biorouter states thinking explicitly, which overrides the server default either
way:

| Effort | Qwen3.8 27B | Gemma 4 31B |
| --- | --- | --- |
| Quick | off | off |
| Normal | as `LLAMACPP_ENABLE_THINKING` says; when on, `reasoning_effort: medium` | as `LLAMACPP_ENABLE_THINKING` says |
| Deep | on, `reasoning_effort: xhigh` | on |

Qwen3.8's template accepts only `low`, `medium` and `xhigh`: `high` makes the server answer HTTP
500, so it is never sent. Its template defaults to `xhigh`, which is why Normal names `medium`
instead of leaving the level out. The warm-up check and session titles always run with thinking
off. Reasoning is shown in the terminal, one block per thinking phase when the phase ends, if
`BIOROUTER_CLI_SHOW_THINKING` is set; the desktop app does not display it.

## Speed

Measured on 2026-10-10 on an Apple M4 Max with nothing else on the GPU (128k window, q8_0 KV
cache, thinking off):

| Model | Cold prompt of ~31,000 tokens | Free text | Rewriting a file |
| --- | --- | --- | --- |
| Qwen3.8 27B | 156 s (about 197 tokens/s) | 18.3 tokens/s | 37.0 tokens/s |
| Gemma 4 31B | 238 s for ~32,000 (about 136 tokens/s) | 16.9 tokens/s | 37.8 tokens/s |

These are cold-prefill timings without a reusable prompt cache. A shared cached prefix can
shorten the wait in later chats. Both models use
self-speculative decoding (`--spec-type ngram-simple`), which drafts from text already in the
conversation and needs no extra model: measured against no speculation, it was 2 to 4 times
faster at rewriting a file and 4 to 10% slower on free text. `LLAMACPP_SPEC_TYPE=none` turns it
off. A machine at the 32 GiB minimum has a fraction of these GPU cores and is several times
slower.

## Related documentation

- [Llama Server model catalog QA checklist](model-catalog-qa-checklist.md): how to verify a
  catalog or build change.
- [Llama Server provider](README.md): the folder index.
- [Ollama provider](../ollama/README.md): the other local provider, serving the same two models.
- [Choosing a model provider](../../getting-started/choosing-a-model-provider.md): where Llama
  Server sits among all the providers.
