# Llama Server model catalog QA checklist

> **What this is.** A repeatable checklist — plus the automated harness that executes
> most of it — for verifying every model in the bundled **Llama Server** (`llamacpp`)
> provider catalog: availability, capability, tool-calling, speed, and robustness.
> **Status:** Current. The test *procedure* below still holds, but two parts of it have
> rotted: the model list in the warning below (and in the commands that use those ids) and
> the expected launch flags in "Robustness signals to watch" were written against an
> earlier pinned llama.cpp build and an earlier `MODEL_CATALOG`. Since then the catalog
> contents, the `--ctx-size` default,
> and the flag used to control thinking have all changed in
> [`crates/biorouter/src/providers/llamacpp.rs`](../../../crates/biorouter/src/providers/llamacpp.rs)
> and [`crates/biorouter/src/providers/llamacpp_sidecar.rs`](../../../crates/biorouter/src/providers/llamacpp_sidecar.rs),
> which are the current truth. Re-derive the model list and the expected launch flags from
> those two files before running the checklist.
> **Audience:** developers working on the `llamacpp` provider and its managed sidecar.

The Llama Server provider gives zero-setup local inference: the desktop app bundles a
pinned `llama-server` binary from llama.cpp and manages it as a sidecar process. Because
llama.cpp releases several times a day with no semantic versioning, a pass through this
checklist is only meaningful against a specific pinned build — and this document carries
no version stamp, so treat every concrete number below as an artifact of the run it was
written from rather than a current expectation.

Everything here exercises Biorouter's *own* provider stack and managed sidecar, not raw
llama.cpp.

> **Warning.** The model list in this document (Qwen3.5 `0.8b / 4b / 9b` and Gemma‑4
> `e2b / e4b / 12b / 26b-a4b`) is the catalog as it stood when the checklist was
> written. Read `MODEL_CATALOG` in
> [`crates/biorouter/src/providers/llamacpp.rs`](../../../crates/biorouter/src/providers/llamacpp.rs)
> for the models actually shipping, and use those ids in the commands below.

## Automated harness (the executable checklist)

[`crates/biorouter/tests/llamacpp_survey.rs`](../../../crates/biorouter/tests/llamacpp_survey.rs)
walks the whole catalog and runs the battery below against each model through the real
`LlamaCppProvider`, writing an incrementally-updated markdown report. Individual model
failures are recorded, never panicked on, so one bad model never aborts the run.

```bash
# All catalog models (~41 GB of downloads on first run):
BIOROUTER_LLAMACPP_BIN=ui/desktop/src/bin/llamacpp/llama-server \
  cargo test -p biorouter --test llamacpp_survey -- --ignored --nocapture --test-threads=1

# A subset (skip the big downloads):
LLAMACPP_SURVEY_MODELS=qwen3.5-0.8b,gemma-4-e2b \
BIOROUTER_LLAMACPP_BIN=ui/desktop/src/bin/llamacpp/llama-server \
  cargo test -p biorouter --test llamacpp_survey -- --ignored --nocapture --test-threads=1
```

- `LLAMACPP_SURVEY_MODELS` — comma list to limit the run (default: all).
- `LLAMACPP_SURVEY_OUT` — report path (default `~/Desktop/llamacpp-survey-report.md`).

Each model is stopped (freed from RAM) before the next loads, so peak memory is one
model at a time. Downloads land in the Biorouter llama.cpp cache (`LLAMA_CACHE` =
`<data dir>/llamacpp/models`) and are reused across runs.

> **Note.** The `~41 GB` first-run download figure and the per-model RAM figures below
> are sums over the catalog as it was at the time of writing. They drift with every
> catalog change — recompute from `MODEL_CATALOG` rather than trusting them.

## The battery (per model)

| # | Dimension | What it checks | Pass criteria |
|---|-----------|----------------|---------------|
| 1 | **Availability** | sidecar `ensure()` + `wait_ready()` (incl. first‑run Hugging Face (HF) download) | reaches `SidecarState::Ready`; load time recorded |
| 2 | **Correctness** | `complete()` on "Reply with exactly: pong" | response contains `pong` |
| 3 | **Thinking‑off** | response `content` is non‑empty | non‑empty (Qwen3.5 with thinking on returns empty `content` and burns the budget on `reasoning_content` — regression guard) |
| 4 | **Tool calling** | `complete()` with a `get_weather` tool | emits a `ToolRequest` (not a prose answer) |
| 5 | **Streaming** | `stream()` yields incremental chunks | `> 1` chunk, non‑empty text |
| 6 | **Speed** | timed ~120‑word generation | tokens/sec from reported `output_tokens` |

### Robustness signals to watch

- **32k context holds**: Biorouter's agent system prompt + tool schemas alone exceed 8k
  tokens; the sidecar defaults to `--ctx-size 32768` with `q8_0` KV cache. A model that
  dies with "Context limit still exceeded after compaction" indicates the ctx default
  regressed. (The default is no longer a flat 32768 — see `configured_context_size` in
  the sidecar source for the value in force.)
- **Tool‑call fidelity under q8_0 KV**: q4 KV‑cache quantization degrades tool calling;
  q8_0 is the chosen floor. Watch dimension 4 across models.
- **No empty answers**: dimension 3 is the canary for the thinking‑off kwarg
  (`--chat-template-kwargs {"enable_thinking":false}`) silently not applying. (The
  sidecar source marks that kwarg form deprecated; `build_args` now emits
  `--reasoning on|off` instead.)
- **Memory headroom**: the 26B‑A4B mixture-of-experts (MoE) model (~16.9 GB GGUF) needs 32 GB+; the 12B needs
  16 GB+. On smaller machines, availability will fail on load, not crash.

## Manual GUI checklist (Playwright or by hand)

Run with the dev app (`ENABLE_PLAYWRIGHT=true npm run start-gui`, see the `debug-ui`
skill; for driving the same dev GUI from a terminal over the Chrome DevTools Protocol,
see [debugging the dev GUI with agent-browser](../../desktop-ui/agent-browser-debugging.md)):

- [ ] **Provider ordering** — Settings → Models → Providers → Manage…: **Llama Server is first**
      under "Local Models", before Ollama; Local group precedes
      Institutional/Commercial.
- [ ] **Configure** — the Llama Server card's *Configure* saves with no API key (Port
      `11543` + 4 advanced options) and marks the provider configured.
- [ ] **Onboarding card** — on the primary welcome screen, `LlamaServerInlineCard` lists
      all catalog models with sizes in its `llamacpp-model-select` dropdown; *Download &
      run* streams progress; auto‑connects when ready.
- [ ] **Switch models** — once configured, Llama Server appears in the bottom‑bar model
      switcher and a chat returns a real response.
- [ ] **Status** — `/llamacpp/status` reflects `starting` → `ready`; the pill in the card
      shows "Running · <model> ready".

## Security invariants enforced in code and unit tests

Do not regress these:

- **Raw HF specs are validated** (`resolve_hf_spec` → `validate_raw_hf_spec`): only
  `owner/repo[:QUANT]` with `[A-Za-z0-9._-]`; no `..`, whitespace, or flag‑shaped values
  reach `llama-server -hf` / the `LLAMA_CACHE` path layout.
- **`LLAMACPP_EXTRA_ARGS` cannot re‑bind the server** (`sanitize_extra_args` +
  re‑asserted trailing `--host 127.0.0.1 --port <port>`): `--host`, `--port`,
  `--api-key`, `--api-key-file`, `--path`, `--rpc` are dropped — config cannot expose
  the unauthenticated sidecar on the LAN.
- **`LLAMACPP_EXTERNAL_HOST`** rejects non‑http(s) schemes and warns loudly when pointed
  at a non‑loopback host (full prompts are sent there unauthenticated).

## Related documentation

- [Environment variables](../../configuration/environment-variables.md) — the
  `BIOROUTER_LLAMACPP_BIN`, `LLAMACPP_EXTRA_ARGS`, and `LLAMACPP_EXTERNAL_HOST` knobs
  this checklist sets.
- [Choosing a model provider](../../getting-started/choosing-a-model-provider.md) — where
  Llama Server sits among the local, institutional, and commercial providers.
- [Debugging the dev GUI with agent-browser](../../desktop-ui/agent-browser-debugging.md)
  — how to drive the dev app for the manual GUI section above.
- [z.ai (GLM) provider](../zai-glm.md) — a sibling provider integration reference whose
  verification section covers the same surfaces for a hosted provider.
