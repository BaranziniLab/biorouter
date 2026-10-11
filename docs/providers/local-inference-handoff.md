# Local inference fix: handoff

> **What this is.** The handoff for the unfinished work on the "local models return JSON instead of
> doing the task" bug: what was found, what is done on the branch, how it was verified, and exactly
> what remains.
> **Status:** Draft; in progress (2026-10-11). Move to `docs/history/` once the PR lands.
> **Checkpoint scope:** Interim checkpoint before area-commit finalization and draft-PR delivery.
> **Audience:** The agent or engineer who picks the branch up.

## Where things stand

The context-override revision `97554ff6` passed its local unit, affected integration and
CI-equivalent gates, and all six area-commit boundaries compiled. Its managed Qwen campaign
completed eight tasks with seven raw artifact passes, four strict quality/workflow passes
and eight recorded task-file scope passes; the failed cases remain explicit. The final
privacy correction is now applied at source `e9b21f6f`, and all six matching compilation
boundaries pass, including four required desktop typechecks and three landing syntax checks.
The temporary checkout was removed; the task branch and index stayed unchanged. Its fresh
workspace run passes 9,328 tests with zero failures and 11 ignored; the core library passes
4,951 with three ignored. Six context/privacy regressions, 55 Ollama tests, 46 core integration,
25 server integration and seven privacy tests pass; overlapping counts are not summed. The
21-gate aggregate, unmodified canonical checks, OpenAPI regeneration/drift, formatting and
five freshly compiled, hash-verified saved executables now pass. Source, HEAD, staged content
and flags stayed unchanged; raw index bytes changed through stat metadata, with no restore
or retesting. All five verified copies are promoted; active receipt `9f4d3202` binds the
relocation `f642c533`. The earlier `04257524` receipt and five binaries are archived under
`bin-archive/042575-20261011T194922Z-cc22274444cf4300ac7ff7e1a7e3e89f/`.
Harness preparation now binds control `e217e8e8`, observer `e2929121` and aggregate `375a3ab7`,
with seven focused mock checks separate from 332 historical fixtures and zero added live cases.
Current Gemma's sealed partial run invokes four of eight requested tasks and completes three:
factorial/note counting pass strict review, loading fails model quality, and querying is
disk-interrupted. Four tasks were not run. All 14 owned processes are absent and serving
state is empty; source/five binaries are unchanged, but disk-floor resource closure fails.
A partial delete-tool name is not proof of execution or mutation. Exact-owned node-cache
pruning passes without new live credit; disk remains below 10 GiB. Six recovery tasks and
the remaining 43-case roster, commit cleanup, durable saving and PR/CI
delivery remain underway. The [validation report](local-inference-validation.md) records
tested source scopes, current outcomes and remaining work; historical results below describe
the original investigation. Original sandbox and wrapper failures are preserved.

- **Branch:** `claude/ollama-llamacpp-json-bug-6cc4c2`.
- **Exact worktree:** `.claude/worktrees/ollama-llamacpp-json-bug-6cc4c2`.
- **Historical investigation base:** `main` at `30e1f06221dfeb2322af99489722a7860783a2f3`.
- **Retained delivery base:** `474f40087490ff3628d88b59be13a9995ff946e3`.
- Fetch and compare the branch and base at phase boundaries. Do not edit the primary checkout.
- The checkpoint history is preserved before organizing the final commits by area. Each code
  boundary must compile and the final tree must match the tested tree.
- Never add an AI `Co-Authored-By` trailer.

## The bug and its root cause

The report came from a user on an Intel Mac running Biorouter 1.92.0 with Ollama (`llama3.1:8b`,
`qwen2.5-coder:7b`).

**The symptom.** Each request sent about 160 KB of prompt, yet Ollama reported
`prompt_tokens + cached = 2050` on every turn. The model replied with invented tool calls written
as JSON text, and recited `workspace__*` tool schemas.

**The truncation mechanism** was confirmed and reproduced under a controlled 4096-default
GGUF configuration:
- Biorouter called Ollama's OpenAI-compatible `/v1/chat/completions`, which **cannot set
  `num_ctx`**.
- With a 4096-token GGUF default, the investigated Ollama version silently kept the first 4
  tokens plus the last ~2046 of Biorouter's ~31-40k-token prompt.
- The model therefore saw only the tail of the alphabetically sorted tool list.
- Biorouter guessed the window as 128k from the model's name, so it never compacted.

**Observed runner behavior.** Ollama 0.40.2 served the two target tags with its MLX runner on
the investigation's Apple Silicon Mac; truncation was not observed for those tags in the
probes. A GGUF variant reproduced the 4096-default failure. These observations do not
establish uniform runner selection or behavior across all platforms and model tags.

**llama.cpp** never truncates silently, but had related failures:
- An over-window prompt returned a 400 that Biorouter misclassified.
- The 32k context tier was below the size of Biorouter's own prompt.
- Four parallel slots shared one KV cache, so two concurrent requests both overflowed.

Full research is in [Research and artifacts](#research-and-artifacts).

## Decisions made with the user

- The recommended Ollama models and the two entries in the managed Llama Server catalog are
  **Qwen3.8 27B** (default) and **Gemma 4 31B**. Ollama also lists other pulled tool-capable tags.
  - Ollama tags: `qwen3.8:27b`, `gemma4:31b` (bare manifest-list tags).
  - Llama Server catalog: `qwen3.8-27b` from `ggml-org/Qwen3.8-27B-GGUF:Q4_K_M` (Qwen publishes no
    GGUF; this is llama.cpp's own conversion) and `gemma4-31b` from
    `google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0`.
  - Retired catalog names alias to the default for chat.
- The user asked for robustness to **every** tool-call protocol and for testing many models through
  Ollama; downloading models is approved.
- The user explicitly asked to test with local models. That overrides the older memory note against
  testing with local Ollama or Llama models.
- Watch RAM and release memory pressure (see [Memory and live-test safety](#memory-and-live-test-safety)).

## What is done

**Phase 1** (backend; implemented, adversarially reviewed, fixed):

- **Ollama provider** (`providers/ollama.rs`, new `providers/formats/ollama.rs`):
  - Requests and streaming:
    - Uses native `POST /api/chat` with `options.num_ctx` and `truncate:false`, plus `keep_alive`
      (default 30m) and `think` mapped from the effort setting.
    - Streams an NDJSON decoder.
    - The `/v1` path is kept only as a fallback when `/api/chat` returns 404, excluding a
      model-not-found response.
  - Context window:
    - Resolution order: `OLLAMA_NUM_CTX`, then the user's context limit, then a memory tier (128k
      at 64 GiB or more, else 64k), capped by `/api/show`'s trained window.
    - The same value is sent on every request kind.
    - The live window is reported through `get_model_config`.
  - Safety checks:
    - A truncation defence fails a turn whose reported prompt is implausibly small.
    - An overflow becomes `ContextLengthExceeded`, with a sentence naming both sizes.
  - Timeouts and retries: idle timeouts, and no retry on a timeout.
  - Other: one host resolver (Ollama's own rules), and a model list that hides tags without tools.
- **Llama Server** (`llamacpp.rs`, `llamacpp_sidecar.rs`, `routes/llamacpp.rs`,
  `ui/desktop/scripts/fetch-llama-server.js`):
  - Catalog and build:
    - The two-model catalog with retired-name aliasing.
    - The pin moves b9611 → **b11429**, with a per-asset SHA-256 check.
  - Context and concurrency:
    - Tiers are 128k at 48 GiB or more, else 64k; the 32k tier is removed, and an OOM floor is
      added.
    - `--parallel 1` plus a client-side slot lease.
  - Security:
    - A per-launch API key file.
    - An `EXTRA_ARGS` allowlist and an environment scrub.
  - Launch, sampling and reporting:
    - `-m` offline launch from the cache.
    - `return_progress`.
    - Per-request thinking (Qwen3.8 never gets `high`) and vendor sampling.
    - `cpu_only` and `below_minimum` suitability statuses.
- **Shared plumbing** (`providers/utils.rs`, `formats/openai.rs`, `retry.rs` with `TimeoutRetry`,
  `toolshim.rs`, `context_mgmt`, `agents/prompt_manager.rs`, `system_small_local.md`):
  - The overflow 400 maps to `ContextLengthExceeded`.
  - `overflow_beyond_compaction` stops the compaction ladder when the prompt and tools alone
    overflow.
  - A truncated-usage floor guard.
  - The toolshim interprets once per reply, refuses non-loopback and cloud interpreters, and logs
    metadata only.
  - The prompt variant is chosen by model size and locality; 27B and 31B models get the Default
    prompt.
  - The overlay wording is code-execution aware.

**Protocol research** (`~/biorouter-runs/local-inference-2026-10/protocols/protocol-matrix.md`):
- 12 Ollama models were probed on the wire.
- Five Ollama handling paths were identified, along with several Ollama 0.40.2 template bugs.
- Requirements R1-R22 and a guarded text-recovery specification came out of it.

**Original phase 2 checkpoint** (implemented, reviewed, fixed; its CI gates were interrupted):

- **Integration:**
  - Classification rows for the new config keys.
  - `errors.rs` no longer reads status codes out of URL port numbers.
  - The IdleOnly read timeout now equals the idle bound.
  - Prompt-progress frames in the real b11429 shape.
  - Live thinking stream.
  - Unsigned thinking is dropped before replay to providers that need signatures.
  - The code-execution "not callable" error now shows the correct call shape.
  - The compaction loop is bounded.
- **Protocol:**
  - R1-R22 on the native path.
  - **Guarded TIER1 text recovery** in `formats/text_tool_calls.rs`, with streaming hold-back in both
    the Ollama and Llama Server providers. It covers L1, L2, L5 and a new L8 (a call wrapped in
    tags). Guards: tools advertised in the request, exact names, required fields and basic
    argument types checked, all or nothing. Recovery checks nested properties/items and
    `additionalProperties: false` to depth 16; it does not resolve `$ref` or evaluate enum,
    composed schemas (`anyOf`/`oneOf`), or other JSON Schema constraints. These stay with the
    tool's normal argument handling.
  - TIER2 runs only in toolshim mode.
  - R8 mitigation: a tool roster in the system prompt, plus a one-time warning.
  - Ollama `:cloud` / `-cloud` models are now Public with no affiliation.
  - 31 fixture-replay tests over 49 real captures (`crates/biorouter/tests/ollama_protocols.rs`,
    `tests/fixtures/ollama/`).
- **Frontend:**
  - Regenerated OpenAPI client.
  - New `/ollama` routes (recommended models, context window, tier).
  - Ollama onboarding:
    - Recommends `qwen3.8:27b` and `gemma4:31b`.
    - Detects pull failures and uses `OLLAMA_HOST`.
    - Handles a remote host without probing it.
  - `cpu_only` and `below_minimum` labels.
  - An Ollama branch in the context gauge.
  - Tests updated for the retired models.
- **CLI and docs:**
  - CLI help text.
  - `models local pull` resolves through `resolve_chat_model_source`, so it never downloads a
    retired model.
  - The configure flow suggests the new models.
  - CLAUDE.md "Llama Server" block rewritten, plus a new Ollama block.
  - New `docs/providers/ollama/`, updated `docs/providers/llama-server/` and getting-started docs.
  - `landing/` docs, mockups and `check-consistency.mjs` updated.

## Historical verification from the original investigation

**Historical live reproduction of the reporter's 4096-default truncation mechanism, using an
Ollama server with that default and GGUF `qwen3.8:27b-q4_K_M`.** The full prompt was seen
(30,754-32,194 tokens, against 2,050 before the fix), the calls were structured, and the
urine-count task was answered correctly (5). This verifies the endpoint/window failure with
a different model and investigation hardware. The reporter's Intel Mac configuration and
the resumed five-slot memory probe are separate verification scopes.

**Live, Ollama MLX on 11434** (urine count and factorial script):

| Model | Urine count (expected 5) | Factorial script |
|---|---|---|
| `qwen3.8:27b` | correct | correct |
| `gemma4:31b` | correct | correct |

The over-window case (`OLLAMA_NUM_CTX=8192`) stops in 1 s with a sentence naming both sizes.

**Live, Llama Server b11429:**
- Both models download through the sidecar's real path, and both pass the `llamacpp_integration`
  live suite (9/9 each).
- Measured on an uncontended M4 Max:

  | Model | Cold prefill of the ~31k-token bootstrap | Free text | File rewrite |
  |---|---|---|---|
  | Qwen3.8 27B | 156 s | 18 tok/s | 37 tok/s |
  | Gemma 4 31B | 238 s | 17 tok/s | 38 tok/s |

- Gemma once failed a code-writing task in code mode (`text_editor.write`). The code-execution error
  message was improved for this, but has not been re-tested live.

**Live, protocol models:**
- `qwen2.5-coder:7b` (the reporter's model) now succeeds through TIER1 recovery (`ollama_live` test
  `a_call_written_as_text`).
- The phase 2 protocol report lists the per-model outcomes; see `phase2-results.txt`.

**Unit and integration tests by area, last runs:**

| Command | Result |
|---|---|
| `cargo test -p biorouter --lib` | 4904 passed, 0 failed (before the last two refactors) |
| `cargo test -p biorouter --test ollama_protocols` | 31 passed |
| `cargo test -p biorouter-cli` | 771 lib passed, 0 failed |
| `cargo test -p biorouter-server --test ollama_routes` / `ollama_routes_tier` / `llamacpp_routes` | all passed |
| Frontend area | lint, format and vitest reported green by its agent |

## Follow-up implementation

- Ollama checks `/api/ps` after the first native reply begins. An automatic window above 64k
  drops to 64k for later requests when the loaded model spills off the GPU, exceeds 75% of
  physical memory, or uses at least 25% while less than 20% remains available. RAM checks are
  local only. The cap is shared across provider rebuilds and every request kind, including
  titles and corrective calls. Explicit `OLLAMA_NUM_CTX` provider settings and
  `BIOROUTER_CONTEXT_LIMIT` process environment overrides are preserved.
- The post-load check cannot prevent an allocation failure before the first reply. Already
  resolved concurrent requests may still use their original window. A healthy observed window
  is not continuously monitored. See [context window and settings](ollama/context-window-and-settings.md)
  for the equal-to-default model-limit provenance caveat.
- Llama startup recovery verifies the request-slot generation before discarding a failed child.
  An old failed startup cannot kill its replacement. A streamed b11429 `Compute error.` discards
  the failed managed backend before releasing its request lease; a late stream error cannot
  discard a replacement. Runtime compute failures return guidance rather than retrying inference.
- Cloud-tag classification is shared by the CLI and provider. Linux NVIDIA VRAM is probed off
  the async worker. Local prompt timestamps use a day boundary to preserve the stable prefix.
- The final privacy guard refuses cloud alternates and discovered remote aliases for a
  reported-Private session before native or remembered OpenAI-compatible inference. It is
  applied and formatted; all six context/privacy regressions and the current workspace run
  pass, as do all 21 current local gates and five fresh executable builds with verified copies.
  The copies are promoted and harness preparation provenance is rebound; full new model
  validation remains pending.
  Earlier passing counts do not validate this later source revision.
- Onboarding ignores stale results after unmount. The context gauge refreshes after a completed
  local turn and ignores results for a switched provider/model.
- Protocol documentation describes L8 recovery, corrective calls, the six-byte truncation
  signal and the eight-token swallowed-call threshold. Basic schema checks do not claim full
  JSON Schema validation.
- A pre-existing Crew transport regression accepted only broken-pipe errors for an exited
  child. The test now accepts the two observed safe terminal paths (broken pipe or SSH EOF),
  retaining exit-status and secret-redaction assertions. Production transport is unchanged.

## Memory and live-test safety

The user's Ollama server runs with **`OLLAMA_NUM_PARALLEL=5`**. Do not change it. In the original
investigation, an automatic 128k GGUF allocation reached 93 GB for `llama3.1:8b` and 125 GB for
`mistral-small3.2:24b`, with severe swap pressure. Do not recreate those allocations.

The old watchdog is stopped. Its script can kill sidecars belonging to other worktrees, so do
not restart it. The resumed harness monitors memory and stops only its own identified processes
and exact test model. Test one model at a time, inspect CPU, memory, swap, disk and thermal state,
and reduce concurrency when resource limits are crossed.

The resumed harness is a separate copy of the original artifacts. Its temporary
`OLLAMA_NUM_CTX: 65536` override has been removed, so it tests Biorouter's automatic window.
Remaining GGUF campaign lanes are scheduled to use an isolated Ollama server with a 4096
default and one parallel slot after its strict start/stop pilot passes; the user's server
setting remains unchanged. A bounded small CPU-only model probe on
that user's five-slot server now passes automatic 128k-to-64k reduction, explicit 128k
preservation, and exact cleanup/source-preservation checks. This is Apple Silicon mechanism
verification, separate from Intel hardware and the isolated campaign lanes; see the
[resumed validation](local-inference-validation.md) for actual outcomes and retained failures.

## Delivery checklist

1. Preserve the completed local gates, fresh binaries and original failure evidence. If live
   verification requires another product fix, rerun affected checks and refresh whole-tree
   gates and binary/source receipts. Finish documentation consistency checks after live results.
2. Run the eight objective tasks for both recommended models on both backends, thinking controls,
   reporter GGUF cases, protocol models, capability rejection, ignored live integration suites,
   the bounded memory probe and the actual self-test workflow. Classify failures and fix product
   defects before rerunning affected checks. Six repeated Llama thinking-disabled tail tasks
   may be omitted only after explicit formal off-mode coverage is verified.
3. Preserve logs, sessions, fixtures, output files, hashes and binary receipts. Review generated
   visualizations as well as their data. Keep the original research folder intact.
4. Organize the checkpoint history by area, compile each code boundary, verify tree identity,
   fetch again, and push with an exact lease against the previously verified remote head.
5. Open the PR against `main`, attach it to this chat, and verify current required CI checks.
   Creating or launching checks is not evidence that they passed.

## Known work outside this PR

- `biorouter run --shared-daemon` can exceed `SUN_LEN` with a long profile-root path and show a
  generic readiness error.
- Separate desktop and CLI processes can each start a sidecar (LC-17).
- The CLI can show `privacy Public` for loopback Ollama until the first private turn.

Do not delete pulled model weights without the user's approval. Build artifacts may be pruned
from this exact worktree after their test evidence is saved; avoid cleaning another worktree.

## Research and artifacts

The original investigation artifacts are in `~/biorouter-runs/local-inference-2026-10/`:

- `research.json`: the five-way research (Ollama and llama.cpp code reviews, the API facts at
  0.40.2 and b11429, model sourcing, the live reproduction).
- `backend-results.json`: phase 1 implement, review and fix reports, including every
  `needs_from_others` item.
- `phase2-results.txt`: phase 2 reports (condensed).
- `protocols/protocol-matrix.md`, `protocols/captures/`, `protocols/probe.py`: the tool-call protocol
  matrix, the raw wire captures, and the probe.
- `livetest/`: the live-test harness, tasks, verifier and fixtures.
- `memwatch/`: the memory watchdog.

## Related documentation

- [Ollama provider](ollama/README.md), including the
  [context window and settings](ollama/context-window-and-settings.md) and the
  [tool-call protocols](ollama/tool-call-protocols.md).
- [Llama Server](llama-server/README.md)
- [Providers index](README.md)
