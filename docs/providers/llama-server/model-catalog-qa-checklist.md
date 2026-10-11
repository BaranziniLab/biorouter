# Llama Server model catalog QA checklist

> **What this is.** A repeatable checklist, plus the automated harnesses that execute most of
> it, for verifying every model in the bundled **Llama Server** (`llamacpp`) provider catalog:
> availability, correctness, tool calling at the size of a real first turn, streaming, speed,
> and the security invariants the managed sidecar must keep.
> **Status:** Current. Rewritten on 2026-10-10 for the two-model catalog and the pinned
> llama.cpp build b11429, checked against
> [`crates/biorouter/src/providers/llamacpp.rs`](../../../crates/biorouter/src/providers/llamacpp.rs)
> and [`crates/biorouter/src/providers/llamacpp_sidecar.rs`](../../../crates/biorouter/src/providers/llamacpp_sidecar.rs),
> which stay the source of truth. A pass is only meaningful against the build it ran on.
> **Audience:** developers working on the `llamacpp` provider and its managed sidecar.

The Llama Server provider gives zero-setup local inference: the desktop app bundles a pinned
`llama-server` binary from llama.cpp and manages it as a sidecar process. llama.cpp releases
several times a day without semantic versioning, so run this checklist whenever
`LLAMA_SERVER_BUILD` or `MODEL_CATALOG` changes. Everything here exercises Biorouter's own
provider and sidecar, not raw llama.cpp.

## What is under test

The catalog holds exactly two models. Read `MODEL_CATALOG` before running anything; the table
below is a snapshot of it.

| Name | Model | Source GGUF | Download | Memory |
| --- | --- | --- | --- | --- |
| `qwen3.8-27b` (default on every machine) | Qwen3.8 27B | `ggml-org/Qwen3.8-27B-GGUF:Q4_K_M` | 19 GB | 32 GiB minimum, 48 GiB recommended |
| `gemma4-31b` | Gemma 4 31B | `google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0` | 18 GB | 32 GiB minimum, 48 GiB recommended |

Each entry also pins the GGUF file's name, byte size and SHA-256. A model already pulled into
Ollama as a GGUF (`qwen3.8:27b-q4_K_M`, `gemma4:31b-it-q4_K_M`, or the bare tag on a machine
that is not Apple Silicon) is reused without a download. Names from earlier catalogs
(`gemma4`, `gemma4-12b`, `qwen3.6` and others in `RETIRED_MODELS`) start the default model when
chatting and delete their own files when deleting.

## Automated harnesses

Two `#[ignore]`d test binaries run real servers and real models. Both need the pinned binary,
which the desktop packaging step (`ui/desktop/scripts/fetch-llama-server.js`) downloads into
`ui/desktop/src/bin/llamacpp/`, and both should run in an isolated root so they do not read or
write your own settings:

```bash
export BIOROUTER_LLAMACPP_BIN=ui/desktop/src/bin/llamacpp/llama-server
export BIOROUTER_PATH_ROOT="$(mktemp -d)"
```

### The integration suite (one model, hard assertions)

[`crates/biorouter/tests/llamacpp_integration.rs`](../../../crates/biorouter/tests/llamacpp_integration.rs)
runs the default model, or `LLAMACPP_TEST_MODEL` (a catalog name or a raw spec), through every
failure the 2026-10 investigation found:

```bash
cargo test -p biorouter --test llamacpp_integration -- --ignored --test-threads=1
LLAMACPP_TEST_MODEL=gemma4-31b \
  cargo test -p biorouter --test llamacpp_integration -- --ignored --test-threads=1
```

| Test | What it proves |
| --- | --- |
| `chat_completion_roundtrip`, `streaming_completion_works` | The model answers, blocking and streamed. |
| `tool_calling_emits_tool_request` | A tool request arrives. See the note below the table: the check also passes on a call recovered from text. |
| `a_bootstrap_sized_prompt_fits_and_still_calls_tools` | A ~28,000-token prompt, the size of Biorouter's real first turn, fits the window and still produces a tool request. |
| `a_long_prefill_streams_progress_and_still_answers` | A cold prefill sends `prompt_progress` frames, which keep the idle timeout fed. |
| `concurrent_requests_queue_instead_of_failing` | Two requests at once both succeed; the second waits. |
| `a_request_queued_behind_a_long_generation_does_not_time_out` | A request waiting for the server's one slot waits inside Biorouter and never times out. |
| `a_thinking_turn_streams_its_reasoning_at_either_level` | Normal and Deep both think and answer. |
| `the_sidecar_refuses_unauthenticated_requests` | Without the per-launch key, `/v1/chat/completions`, `/props` and `/v1/models` answer 401; `/health` answers 200. |
| `an_over_window_prompt_is_a_context_error` | An over-long prompt is a context-length error naming both sizes, never a silent cut. |
| `switching_models_restarts_sidecar` | A model switch restarts the server. |

⚠ The provider runs every reply through Biorouter's text recovery before a test sees it, so a
reply that is a whole call written as JSON text (the 2026-10 failure itself) reaches both tool
tests as a tool request and passes. None of these checks (the two tests, and the survey's rows
4 and 5 below) looks at the `recovered_from_text` marker or prints logs, so a pass does not by
itself prove llama-server parsed the call. In a real chat on the model, Biorouter's log tells
them apart: a recovered call logs `recovered tool calls the model wrote as text`, with the model
and the leak format.

Measured on 2026-10-10 on an Apple M4 Max: Qwen3.8 27B passed 11 of 11 in 450 s. The binary
leaves its last server running when it exits, because the sidecar manager lives in a static;
the next Biorouter process to start a model reaps it.

### The survey (every model, a report)

[`crates/biorouter/tests/llamacpp_survey.rs`](../../../crates/biorouter/tests/llamacpp_survey.rs)
walks the whole catalog and runs the battery below against each model, writing a Markdown
report as it goes. A model that fails is recorded, never panicked on, so one bad model never
stops the run.

```bash
# Every catalog model (~37 GB of downloads on a first run):
cargo test -p biorouter --test llamacpp_survey -- --ignored --nocapture --test-threads=1

# One model:
LLAMACPP_SURVEY_MODELS=qwen3.8-27b \
  cargo test -p biorouter --test llamacpp_survey -- --ignored --nocapture --test-threads=1
```

- `LLAMACPP_SURVEY_MODELS`: a comma-separated list to limit the run (default: every model).
- `LLAMACPP_SURVEY_OUT`: the report path (default `~/Desktop/llamacpp-survey-report.md`).
- `LLAMACPP_SURVEY_READY_SECS`: how long a download may make no progress (default 8 hours).

Each model is stopped before the next loads, so only one is in memory at a time. Downloads land
in the shared model store (`LLAMA_CACHE` is `OLLAMA_MODELS`, or `~/.ollama/models`) and are
reused by later runs, by the app and by Ollama's own tooling.

## The battery, per model

| # | Dimension | What it checks | Pass criteria |
| --- | --- | --- | --- |
| 1 | **Availability** | The sidecar starts the model, downloading it first if needed | Reaches `SidecarState::Ready`; load time recorded |
| 2 | **Correctness** | "Reply with exactly the word: pong" | The reply contains `pong` |
| 3 | **Thinking off** | The same reply's text | Not empty. With thinking on, a reasoning model can spend a small budget on reasoning and return no text; the warm-up path turns thinking off for this reason |
| 4 | **Tool calling** | A request with a `get_weather` tool | A tool request, not prose (structured, or recovered from a whole-reply JSON call; see the note above) |
| 5 | **Bootstrap-sized prompt** | The same request behind a ~30,000-token system prompt | Still a tool request; prefill rate recorded |
| 6 | **Streaming** | A streamed count to five | More than one chunk, non-empty text |
| 7 | **Speed** | A timed generation | Tokens per second from the reported output tokens |

Reference numbers, measured on 2026-10-10 on an M4 Max with nothing else on the GPU (128k
window, q8_0 KV cache, thinking off): Qwen3.8 27B prefilled ~31,000 tokens in 156 s and wrote
free text at 18.3 tokens per second; Gemma 4 31B took 238 s and wrote at 16.9. A machine at the
32 GiB minimum has a fraction of those GPU cores and will be several times slower.

## Launch flags to expect

The sidecar logs its argv at start (`Starting llama-server (b11429) ...`). Check it against
this list:

- `-m <file>` when the model is cached (the catalog's Hugging Face file first, then a GGUF in
  Ollama's store), and `-hf <spec>` only when nothing is cached.
- `--alias <name> --ctx-size <n> --parallel 1 --jinja --no-ui --no-slots --no-mmproj`.
- `--ctx-size` 131072 with at least 48 GiB of GPU-addressable memory, else 65536, unless
  `LLAMACPP_CONTEXT_SIZE` pins it. Never 32768: Biorouter's instructions and tools alone take
  ~31,000-40,000 tokens. After an out-of-memory start the window steps down to 65536 and no
  further.
- `--cache-type-k q8_0 --cache-type-v q8_0` (q4 KV degrades tool calling).
- `--reasoning off`, or `on` with `LLAMACPP_ENABLE_THINKING=true`. Biorouter's own requests
  state thinking per request anyway.
- `--spec-type ngram-simple`, each catalog model's measured default (`LLAMACPP_SPEC_TYPE=none`
  removes it).
- Allowlisted `LLAMACPP_EXTRA_ARGS`, then `--api-key-file <run dir>/<pid>.key`, then
  `--host 127.0.0.1 --port <port>` last.

## Manual GUI checklist

Run with the dev app (`ENABLE_PLAYWRIGHT=true npm run start-gui`, see the `debug-ui` skill; to
drive the same dev GUI from a terminal over the Chrome DevTools Protocol, see
[debugging the dev GUI with agent-browser](../../desktop-ui/agent-browser-debugging.md)):

- [ ] **Provider ordering**: Settings → Configure providers: **Llama Server is first** on the
      Local tab, before Ollama, and the tabs run Local · Institutional · Public.
- [ ] **Configure**: the Llama Server card saves with no API key and marks the provider
      configured.
- [ ] **Onboarding card**: `LlamaServerInlineCard` lists both catalog models with their sizes
      and a suitability note; *Download & run* shows progress and connects when ready.
- [ ] **Suitability**: the card shows the route's suitability message for this machine. Under
      32 GiB it says the model is below the minimum; on an Intel Mac it says Llama Server has no
      GPU acceleration there. Neither machine is offered a different model.
- [ ] **Switch models**: once configured, Llama Server appears in the model switcher and a chat
      returns a real response.
- [ ] **Status**: `/llamacpp/status` moves from `starting` to `ready`, and the card shows the
      model running.

## Security invariants

Each is enforced in code and covered by a unit test. Do not regress them:

- **The server needs a key.** Every launch generates a fresh random API key, writes it to a
  file only the user can read, and passes it with `--api-key-file`. The loopback bind alone was
  not enough: llama-server reflects any `Origin` in its CORS answers, so any web page the user
  opened could drive the model, and `/props` leaks the user's home directory.
- **`LLAMACPP_EXTRA_ARGS` is an allowlist** (`ALLOWED_EXTRA_FLAGS`, plus `--spec-*` without the
  flags that load or download a draft model). It replaced a blocklist that missed `--tools`,
  which turns on llama-server's built-in shell and file tools for any caller. Flags are matched
  the way llama.cpp parses them: `_` becomes `-` in a `--` flag, `--flag=value` is split, and a
  argument arity and value-token shape are checked. Missing or flag-shaped required values
  are rejected; `--flash-attn` values get an explicit enum check. Most numeric and range
  constraints remain for llama-server to validate.
- **The bind and the key file come last** in the argv, after the user's arguments, as a second
  layer behind the allowlist.
- **The child's environment is scrubbed.** Every `LLAMA_*` variable (llama-server reads most
  flags from `LLAMA_ARG_*`) and every Hugging Face token is removed; only `LLAMACPP_HF_TOKEN`
  reaches the server, as `HF_TOKEN`.
- **Raw Hugging Face specs are validated** (`validate_raw_hf_spec`): only `owner/repo[:QUANT]`
  with `[A-Za-z0-9._-]`, no `..`, whitespace or flag-shaped values, so nothing reaches
  `llama-server -hf` or the cache layout that could escape it.
- **The pinned binary is verified.** `ui/desktop/scripts/fetch-llama-server.js` checks every
  downloaded archive and the license against pinned SHA-256 digests before extracting, and
  refuses an archive with no pinned digest.
- **`LLAMACPP_EXTERNAL_HOST`** rejects schemes other than `http` and `https`, and warns loudly
  when pointed at a host that is not this machine, because the whole conversation is sent there.

## Related documentation

- [Llama Server model catalog](model-catalog.md): the two models, the launch order, and the
  context, request and thinking policy this checklist verifies.
- [Environment variables](../../configuration/environment-variables.md): the `LLAMACPP_*` and
  `BIOROUTER_LLAMACPP_BIN` settings this checklist sets.
- [Choosing a model provider](../../getting-started/choosing-a-model-provider.md): where Llama
  Server sits among the local, institutional and commercial providers.
- [Ollama provider](../ollama/README.md): the other local provider, which serves the same two
  models.
- [Debugging the dev GUI with agent-browser](../../desktop-ui/agent-browser-debugging.md): how
  to drive the dev app for the manual GUI section.
