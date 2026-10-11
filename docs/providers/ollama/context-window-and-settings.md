# Context window and settings

> **What this is.** How Biorouter builds a request to an Ollama server: the native API it
> uses, how the context window is chosen, how a cut prompt is caught, and every setting that
> changes the request.
> **Status:** Current. Checked against `crates/biorouter/src/providers/ollama.rs`,
> `crates/biorouter/src/providers/formats/ollama.rs` and
> `crates/biorouter-server/src/routes/ollama.rs` on 2026-10-10; the measurements are from
> Ollama 0.40.2 on an Apple M4 Max with 128 GiB of memory.
> **Audience:** end users running local models through Ollama, and developers changing the
> Ollama provider.

In October 2026 a user reported that with Ollama the agent "kept returning JSON instead of
doing the task". The cause was not the model. Biorouter sent its requests to Ollama's
OpenAI-compatible endpoint, which cannot set the context window, and the user's Ollama used a
4096-token window. Ollama then kept the first 4 and the last ~2046 tokens of Biorouter's
~31,000-40,000-token prompt, answered with HTTP 200, and logged one warning. The model saw only
the tail of the tool list, so it wrote invented tool calls as JSON text. Biorouter believed it
had a 128k window, so nothing noticed. The native context settings and checks below address
this failure; their limits on older servers and the compatibility fallback are described below.

## What a request looks like

Every request is a `POST /api/chat` with:

| Field | Value |
| --- | --- |
| `model` | The configured model tag, such as `qwen3.8:27b`. |
| `messages`, `tools` | The conversation and the tool schemas. No tools in chat-only mode. |
| `stream` | Always `true`, also for compaction summaries and session titles, which are collected into one message. |
| `options.num_ctx` | The context window (see below). |
| `options.num_predict` | The output budget the model config pins, or a ceiling (see below). |
| `options.temperature` | Only when the model config pins one. Otherwise the model's own Modelfile sampling applies. |
| `truncate` | `false`, so an over-long prompt is refused instead of cut. |
| `keep_alive` | How long the model stays loaded after the request (see below). |
| `think` | Only for a model whose capabilities include `thinking` (see below). |

### Why not the OpenAI-compatible endpoint

Ollama's `/v1/chat/completions` builds its options from a fixed list (`stop`, `max_tokens`,
`temperature`, `seed`, the two penalties, `top_p`) and ignores everything else, including
`options.num_ctx`. Measured on Ollama 0.40.2: `/v1` reported 2,050 prompt tokens for the same
request with and without `options.num_ctx`, while `/api/chat` with `options.num_ctx` processed
all 15,950. Without `num_ctx`, Ollama also sizes the cache for the model's whole trained window
times its parallel slots; on a 128 GiB machine that meant requests for 655,360 tokens of cache,
out-of-memory failures, and waits of 9 to 15 minutes.

Biorouter still falls back to `/v1/chat/completions` in one case: when `/api/chat` answers
HTTP 404 with anything other than "model not found". That is a server that is not Ollama at
all, behind a custom provider. The switch lasts for the rest of that provider instance, and
on that path every reply is held back until its prompt size has been checked (see
[when a server cuts the prompt anyway](#when-a-server-cuts-the-prompt-anyway)).

## How the context window is chosen

`num_ctx` is chosen per model and sent on every request: turns, compaction summaries and
session titles alike. A different `num_ctx` makes Ollama reload the model, so Biorouter keeps
the chosen number unless its post-load memory check reduces an automatic window. In order of precedence:

1. `OLLAMA_NUM_CTX`, when it is set to a positive number.
2. A context limit you set for the model: `BIOROUTER_CONTEXT_LIMIT`, or a limit that differs
   from the one Biorouter guesses from the model's name.
3. Otherwise, by memory: **131,072 tokens** (128k) with at least 64 GiB of GPU-addressable
   memory, else **65,536 tokens** (64k).

Whatever is chosen is capped at the window the model was trained for, read from Ollama's
`/api/show` (`model_info["<architecture>.context_length"]`), because Ollama clamps to it
anyway. The automatic choice is never below 64k, because Biorouter's own instructions and
tool schemas take about 31,000 tokens with the default extensions and up to about 40,000 with
more. A model trained on a smaller window than that cannot run Biorouter's full prompt at
all.

The window Biorouter sends is also the window it reports to compaction and to the context
gauge. Before this change both trusted a guess from the model's name.

> **Note.** GPU-addressable memory means unified memory on Apple Silicon, system RAM on an
> Intel Mac (Ollama's x86_64 build has no Metal), and the detected VRAM on a discrete GPU. On
> Linux it is an NVIDIA GPU's VRAM, read with `nvidia-smi`, and never system RAM: Ollama ships
> CUDA there, so a 64 GiB machine with a 12 GB GPU is sized by the 12 GB. A Linux machine with
> no NVIDIA GPU, or a GPU Biorouter cannot measure, gets 64k. The Llama Server provider chooses
> 128k from 48 GiB, and on Linux it measures its own bundled CPU build by system RAM.

A custom provider declared with the `ollama` engine does not read `OLLAMA_NUM_CTX`, which
describes the built-in provider's server. It uses `BIOROUTER_CONTEXT_LIMIT` or the automatic
choice.

### Checking memory after the model loads

After the first native reply starts, Biorouter reads Ollama's
[`GET /api/ps`](https://docs.ollama.com/api/ps) for that model's loaded `size`, `size_vram` and
`context_length`. It checks once per successfully observed window. If the automatic window
is above 64k, it reduces later requests to **65,536 tokens** when the model spills outside
GPU memory (`size_vram < size`), takes more than 75% of this computer's physical RAM, or
takes at least 25% while less than 20% of RAM remains available. The RAM checks apply only
to a server on this computer; a remote server is judged only by its own reported GPU spill.
Ollama versions and runners may report either the request window or the cache across all
parallel slots; Biorouter accepts a positive whole multiple of the requested window and keeps
the per-request window for compaction.

The first request finishes with the original window. The next request reloads once at 64k;
turns, corrective retries, summaries, session titles and the context gauge then use the
reduced window. The reduction is shared across provider instances for that server and model
for the rest of Biorouter's process. If 64k still spills or leaves too little RAM, Biorouter
warns once and keeps 64k so its instructions and tools can fit. A failed probe, an older
server that does not report these fields, or another model's allocation never changes the
window.

This check preserves an explicit `OLLAMA_NUM_CTX` provider setting, whether saved in the
configuration or supplied through the environment, and a `BIOROUTER_CONTEXT_LIMIT` process
environment override. Numeric model limits different from the name-based default are also
preserved. A numeric limit equal to that default lacks override provenance and is treated
as automatic; configured model-picker, API, and worker values can have this ambiguity.
Use `OLLAMA_NUM_CTX` or a `BIOROUTER_CONTEXT_LIMIT` environment override to remove it. A saved
`BIOROUTER_CONTEXT_LIMIT` value alone does not provide that protection. Provider rebuilds
retain the original configuration rather than mistaking a
reported automatic window for a user override. Biorouter never changes
the Ollama server's settings. The warning names `OLLAMA_NUM_PARALLEL`, because parallel slots
multiply the cache allocation, and `OLLAMA_NUM_CTX`, which lets you choose a window yourself.
Since the check runs after loading, a load that fails before returning a reply still needs a
smaller explicit window or fewer parallel slots.

## When the prompt does not fit

With `truncate: false`, Ollama answers an over-long prompt with HTTP 400. Biorouter turns that
into a context-length error that names both sizes and the setting to raise. When the
conversation can be shortened, the agent compacts it and retries. When Biorouter's
instructions and tool schemas alone fill the window, compacting cannot help, so the turn stops
at once with one sentence that says so: raise the window, or turn off extensions you are not
using.

Measured with `OLLAMA_NUM_CTX=8192`: Ollama refused a 29,187-token prompt, and the turn
stopped after that one request, in about one second, with the message that the instructions
and tool descriptions alone take about 28,700 of the 8,192 tokens.

## When a server cuts the prompt anyway

`truncate: false` is honoured from Ollama 0.12.6. An older Ollama, or a non-Ollama server on
the `/v1` fallback, may still cut silently. When usable whole-prompt counts are available,
Biorouter checks them with a bytes-per-token heuristic. For a request with more than 20 KB
of text, a count implying more than 12 bytes per token is treated as a cut: the turn fails
and no tool call from it runs. Where a server may cut and its count can be judged, answer
text is held until this check passes. A count shaped like Ollama's cut (half a power-of-two
window plus 1 to 8 tokens, such as 2,050 of 4,096) is held to 6 bytes per token instead,
because a cut to half the window is not always far from a real ratio: `lfm2:24b` reported
16,387 prompt tokens for a 38,570-token request, about 9.8 bytes per token, and answered as if
nothing had happened. The error says that the server cut the prompt, estimates both sizes,
and says what to change. The reporter's requests were 160 KB reported as 2,050 tokens, about
78 bytes per token. The thresholds reflect measured prose, code and JSON; they are not a
tokenizer guarantee and cannot detect every cut. A missing prompt count cannot be judged,
and before Ollama 0.4.0 the count omits cached tokens, so the check is disabled.

| Ollama version | What happens |
| --- | --- |
| 0.12.6 or later, or a source build reporting `0.0.0` | Over-long prompts are refused; replies stream as they arrive. |
| 0.4.0 to 0.12.5 | Prompts are cut, but the reported count shows it. Replies are held until the size check passes, and a warning says to update Ollama. |
| Before 0.4.0 | The reported count leaves out cached tokens, so a cut cannot be told from a cache hit and nothing is judged. A warning says to update Ollama. |
| `/v1` fallback | Treated like a server that cuts: replies are held and judged. |

## Thinking

`think` is sent only to models whose `/api/show` capabilities include `thinking`; the others
answer `think: true` with HTTP 400. What is sent follows the reasoning-effort control:

| Effort | `think` |
| --- | --- |
| Quick | `false` |
| Normal | Nothing, so the model's own default applies, unless `OLLAMA_THINK` is set |
| Deep | The deepest level the model declares: `xhigh` for Qwen3.8, `true` for a model that only takes `true` or `false`, such as Gemma 4 |

A level is sent only if the model declares it. Qwen3.8's renderer rejects `high`, so `high` is
never forwarded. A request that pins an output budget under 1,024 tokens gets `think: false`
unless the effort is Deep: `biorouter configure` tests a model with a 50-token budget, and at
its default thinking `qwen3.8:27b` spent all 50 tokens reasoning and returned no answer. With
`think: false` the same test made its tool call in 26 tokens. Session titles always send
`think: false`.

A reply that contains only reasoning, with no text and no tool call, is never taken as an
answer. When the request offered tools, Biorouter asks the model once more, because a model
that thinks may also have written a tool call Ollama could not parse; if that fails too, or no
tools were offered, the turn ends with an error saying the model returned reasoning only. See
[tool-call protocols](tool-call-protocols.md#how-biorouter-handles-each-outcome).

## Output budget, keep-alive and timeouts

- **Output budget.** A turn sends the `num_predict` the model config pins. Otherwise it sends
  16,384, or 32,768 when the model may think, so a model that starts repeating itself stops.
  A title is capped at 128.
- **Keep-alive.** `keep_alive` is `OLLAMA_KEEP_ALIVE`, default `30m`. Ollama's own default is
  5 minutes, after which a 17-19 GB model unloads and the next turn pays the load plus a full
  re-read of Biorouter's ~30,000-token prompt. A number is seconds, and `-1` keeps the model
  loaded.
- **Timeouts.** `OLLAMA_TIMEOUT` (default 600 seconds) limits silence between pieces of the
  reply, not the whole reply, because a local model can stream for longer than any fixed
  deadline. A timeout is never retried: the original inference may still be running, so
  resending can duplicate work or queue behind it, depending on Ollama's parallel slots.
  Connection failures before a reply starts are still retried.

> **Note.** The first piece of a reply arrives only after the model has read the whole prompt.
> On a slow machine a cold ~31,000-token prompt can take longer than `OLLAMA_TIMEOUT`; raise it
> if a first reply times out.

### A timeout with two models loaded and parallel slots

Ollama sizes a model's cache as `num_ctx` times its parallel slots (`OLLAMA_NUM_PARALLEL`, set
on the Ollama server, not in Biorouter). Measured on Ollama 0.40.2: with 5 slots, Biorouter's
64k window became a 327,680-token load of 44.6 GiB. When two such models are loaded, each can
wait for memory the other holds, and the only thing the user sees is a 600-second timeout with
no reason given. If a first reply times out while another model is loaded, unload it
(`ollama stop <model>`), or run the Ollama server with fewer parallel slots, such as
`OLLAMA_NUM_PARALLEL=1`.

## Models

Biorouter recommends two local models for Ollama:

| Tag | Model | Download |
| --- | --- | --- |
| `qwen3.8:27b` | Qwen3.8 27B, the default | about 18 GB |
| `gemma4:31b` | Gemma 4 31B | about 19 GB |

Both need about 32 GiB of GPU-addressable memory; on a smaller machine Biorouter still uses
them and logs a warning that replies will be slow or the model may not load. In live agent
runs in October 2026 both made structured tool calls throughout, answered a five-occurrence count over
clinical notes correctly, and wrote and ran a small script. The bare tags are manifest lists:
Ollama pulls an MLX build on Apple Silicon and a GGUF build elsewhere. `qwen3.8:27b` needs
Ollama 0.32.12 or later; `gemma4:31b` needs 0.31.0 for its MLX build and 0.30.9 for its GGUF
build. An older Ollama refuses the pull. The GGUF-only tags
`qwen3.8:27b-q4_K_M` and `gemma4:31b-it-q4_K_M` also work, and the Llama Server provider can
start from either.

The model list shows the models you have pulled, minus any whose `/api/show` capabilities do
not include `tools`, because Biorouter sends tools on every turn and such a model fails the
first one. Your configured model always stays listed. `biorouter configure` lists the two
recommended models first when you have pulled them, and tells you which to pull when you have
pulled neither. It lists cloud models (such as `gemma4:31b-cloud`) last, labelled as sent to
ollama.com, and never counts one as a recommended local model. Other tool-capable models stay
listed, but many do not perform reliably as an agent. See
[tool-call protocols](tool-call-protocols.md) for which, and why.

## The desktop app

Two daemon routes give the desktop app the provider's own answers, so it never keeps a second
copy of them. Both sit behind the daemon's secret key.

| Route | Answer |
| --- | --- |
| `GET /ollama/recommended` | The two recommended models (default first, with download sizes), this machine's memory as the provider sizes the window by it, a suitability rating for the recommended models (`suitable`, `below_minimum`, `cpu_only` for an Intel Mac, or `unknown_resources`), the resolved `OLLAMA_HOST`, whether `OLLAMA_HOST` is set, and the privacy `tier` a chat on the default model gets from that server. |
| `GET /ollama/context_window?model=<tag>` | The `num_ctx` a chat on that model sends to the configured server, built exactly as a chat builds it. `null` when the server cannot describe the model (unreachable, not Ollama, or not pulled). |

The onboarding card (`OllamaInlineCard`) reads the first route:

- It uses the configured `OLLAMA_HOST`, and never overwrites one you set. It writes the default
  only when none is set.
- Whether the server is "this computer", and private, comes from the route's `tier`, the
  provider's own answer. The card does not decide it from its own list of host names.
- A failed pull is reported with Ollama's own error.
- The app can probe only `http://127.0.0.1` (its content security policy allows no other
  address), so a server on another machine is not probed and no download is offered, since that
  would land on this machine's Ollama. The card asks you to run `ollama pull <tag>` on that
  server, and lets you connect to it. A host of `0.0.0.0` or `[::]` (Ollama listening on every
  interface) is probed as `127.0.0.1`.

The context gauge reads the second route (`utils/liveContextWindow.ts`), in place of the guess
from the model's name ("qwen" read as 128k) that hid the original bug, and ignores an answer for
a model the chat has since switched away from.

## Settings

| Setting | What it does | Default |
| --- | --- | --- |
| `OLLAMA_HOST` | The server. Read exactly as Ollama's own client reads it: a value without a scheme means `http` and port 11434 unless given; `http://` without a port means 80, `https://` 443. | `localhost` |
| `OLLAMA_TIMEOUT` | Seconds of silence before a request is abandoned. | `600` |
| `OLLAMA_NUM_CTX` | The context window to ask for, in tokens. | automatic |
| `OLLAMA_KEEP_ALIVE` | How long the model stays loaded: a duration such as `30m`, or seconds (`-1` is forever). | `30m` |
| `OLLAMA_THINK` | What Normal effort sends as `think`: `true`, `false`, or a level the model declares. | the model's default |
| `BIOROUTER_CONTEXT_LIMIT` | A context limit for the model; the window for a custom `ollama`-engine provider. | none |

`OLLAMA_HOST`, `OLLAMA_TIMEOUT` and `OLLAMA_NUM_CTX` appear in the Ollama provider's settings
form (**Settings** > **Models** > **Configure providers**). Set `OLLAMA_KEEP_ALIVE` and
`OLLAMA_THINK` in `config.yaml` or as environment variables; for every `OLLAMA_*` key an
environment variable wins over `config.yaml`. `BIOROUTER_CONTEXT_LIMIT` is read from the
environment only.

## Privacy tier

An Ollama on this machine (a loopback `OLLAMA_HOST`) is a private model, so a private chat may
use it. An `OLLAMA_HOST` that points at another machine makes the provider public, because
Biorouter cannot vouch for where that server sends the conversation.

An Ollama **cloud model** is public even on this machine's Ollama, because Ollama answers it by
forwarding the conversation to ollama.com. Biorouter recognises one by its tag (`cloud`, or a
tag ending in `-cloud`, such as `gpt-oss:120b-cloud`) or by Ollama reporting that it serves the
model from a remote host, which also catches a cloud model copied under another name.

## When the model writes JSON instead of calling tools

Work through these in order:

1. **Update Ollama** to 0.32.12 or later, which `qwen3.8:27b` needs. Versions before 0.12.6
   also cut an over-long prompt even when told not to.
2. **Read the error.** A cut or over-long prompt now stops the turn with a message that names
   both sizes. If it says the instructions and tools alone fill the window, raise
   `OLLAMA_NUM_CTX` or turn off extensions you are not using.
3. **Use a recommended model**, `qwen3.8:27b` or `gemma4:31b`. Some models never make a
   structured tool call through Ollama, whatever the prompt: `qwen2.5-coder:7b` writes every
   call as JSON text. Biorouter runs such a call only when the whole reply is the call and it
   passes every guard: each name exactly matches a tool offered in the request, and argument
   objects pass required-field and directly declared type checks, including nested
   properties/items and `additionalProperties: false` to depth 16. Recovery does not resolve
   `$ref` or evaluate enum, composed schemas, or other JSON Schema constraints; those remain
   for the tool's normal argument handling. A call that follows prose stays text. See
   [tool-call protocols](tool-call-protocols.md).

## Verifying a change

```bash
cargo test -p biorouter --lib ollama
cargo test -p biorouter --test ollama_live -- --ignored --test-threads=1
```

The second command needs a running Ollama with both recommended models pulled. It runs the real
`biorouter configure` probe on each model. `OLLAMA_TEST_MODELS` (comma-separated) replaces the
model list, and `OLLAMA_HOST` picks the server. Point `BIOROUTER_PATH_ROOT` at a scratch
directory so the test does not read your own `config.yaml`.

## Related documentation

- [Tool-call protocols](tool-call-protocols.md): what each model returns for a tool call, and
  which models to use.
- [Ollama provider](README.md): the folder index.
- [Llama Server provider](../llama-server/README.md): the bundled alternative running the same
  two models.
- [Environment variables](../../configuration/environment-variables.md): where these settings
  sit among all the others.
