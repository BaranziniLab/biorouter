# Local inference fix: handoff

> **What this is.** The implementation and validation record for local models returning
> JSON instead of performing the task, with source scopes and unresolved quality limitations.
> **Status:** Historical record — local-inference validation concluded 2026-10-12 UTC (2026-10-11 PDT);
> implementation is in unmerged [PR #393](https://github.com/BaranziniLab/biorouter/pull/393).
> Final-head CI is recorded by [the PR's current-head checks](https://github.com/BaranziniLab/biorouter/pull/393/checks).
> **Audience:** maintainers tracing the implementation, verification and delivery requirements.

## Where things stand

### Survey-fixture-only current generation

The current local source is commit `dfd0db50022be648303e79d174132616376ef20b`, identity `2ebc7e8b990e161f236e8fd1a08cbeb38dc14d6f24f01b7d4496b7e22c65cd3a`. Only the survey repeat count changes from 560 to 1,000 with truthful end-to-end labeling; acceptance predicates and production source are unchanged. The production-except-survey equality is `77bed48e`, witnessed by bridge `250c5e8e`. Actual promotion `ccfb77e8` binds current receipt `ddbf3422`: four unchanged executables are Cargo Fresh=true and the survey is Fresh=false (`ac42d6f6`). The complete prior `9f4d3202` five-copy generation and its raw receipt are archived. The previous e9 full-suite/gate checks retain their tested scope; this bridge does not claim a new full production-suite rerun. Current aggregate is `d3541e51`, with control `e217e8e8`, observer `e2929121` and historical fixtures unchanged.

Privacy-generation source `e9b21f6f` passes all six compilation boundaries, four required desktop typechecks,
three landing syntax checks, 21 local gates, 9,328 workspace tests (zero failed, 11 ignored),
4,951 core-library tests and the affected context/privacy/server integrations. Counts
overlap and are not summed. At that stage the five freshly built saved executables were promoted with
receipt `9f4d3202`; relocation `f642c533` and the prior five-copy archive preserve lineage.
That preparation binds control `e217e8e8`, observer `e2929121` and aggregate `375a3ab7`; seven
focused mocks remain separate from 332 historical fixtures and add no live credit.

Interim H1 passes all 16 expected PR checks. The original native Windows lock-contention
failure remains preserved; one unchanged-head retry passes without establishing its cause.
The [validation report](local-inference-validation.md) records platform/phase counts and
configured skips. H1 is historical evidence, not a final-head CI claim.

Current Gemma's original G attempt requests eight tasks, invokes four and gets three complete responses: factorial/note
counting pass strict review; LOAD fails quality; QUERY is disk-interrupted; four tasks are
not run. Its original disk-floor closure failure remains. Distinct R6 attempts two of six with
zero raw/strict passes: LOAD finishes but omits required clinical/fictional facts and uses
invalid schema/workflow; submitted stub bytes persist correctly. QUERY times out with no
answer and a user-gated KB-delete request; complete arguments/approval/result are absent,
so deletion execution is unproven. R6's other four tasks are not run. Owner comparison
`3c7a90c5` separately finds the original 76 snapshot files unchanged with no extras/errors.
R6 cleanup passes (`6c8e3e01`) while actual campaign/watcher exits remain 1 (`38a4ce30`).
Neither cleanup nor transport success establishes model clinical quality or a whole-pair pass.

Historical managed Qwen `97554ff6` retains 7 of 8 raw artifact passes, 4 of 8 strict quality/
workflow passes, 8 of 8 recorded file-scope passes and all documented failures. Later isolated
CURATE/LABS/DEBUG/VIZ and the fixed campaign (43 launches, 46 invocations) retain independent
source/run identities.
A relocated CURATE seed must prove 75 exact files and one registry with two runtime-path
rewrites, while preserving the original snapshot; it is not 76 exact runtime copies.

Final roster outcomes:

### Closed task outcomes and finite roster through all 45 sequences

This is the closed model-roster record; delivery verification requirements are recorded below. All 45 original listed commands have closed outcomes. The corrected survey adds one actual command/functional launch/attempt: 46 actual commands, 44 functional runner launches, 47 task/test attempts and two passing lifecycle controls. Controls, nested effects and historical CI are excluded from task/test counts.

The later Gemma task attempts remain distinct from G and R6:

| Separate attempt | Closed outcome | Material limitation |
| --- | --- | --- |
| LABS | Strict quality/workflow/recorded scope PASS | Two recovered tool errors; saved snapshot follows DEBUG startup |
| DEBUG | Supplemental strict quality/workflow/recorded scope PASS; recorded eight tests OK | Original wrapper failure and missing canonical exports remain; closed DB-copy export supplies the supplement, without a model rerun |
| VIZ | Static numerical/source/workflow/final/recorded scope PASS; exact five means, no added units | Browser rendering UNVERIFIED after policy rejection; historical Chrome evidence covers a different chart |
| Seeded CURATE | Timeout, no final and no completed KB operation: task/workflow FAIL | One completed TODO; gated delete intent has no complete arguments/approval/result or execution proof |

The CURATE seed preserves 75 files exactly and rewrites only two runtime paths in one registry. Original R6 files remain unchanged. Of 77 seeded records, 72 remain unchanged, five Soul metadata records change and four nonempty Git objects are new, with none removed; startup recovery is an inference, not model curation or demonstrated deletion. Source/raw/schema/index/log/config/registry/fixtures remain preserved. Its authenticated GET-only serving supplement is separate from the canonical timeout summary. Historical tail3 explicit toolshim=false was not attested because its caller misspelled the environment variable; source default=false is qualified. Fresh VIZ/CURATE callers use the correct name, without per-request wire proof.

| Finite sequences | Actual closed outcome | Count and scope |
| --- | --- | --- |
| 1–24 | 24 effective model/function cases PASS | 25 actual formal attempts: 24 PASS and the original undersized Gemma survey FAIL; original failed attempt retained |
| 25 | Owned isolated daemon startup PASS | One lifecycle control; zero functional/model-inference credit; same-owned stop is separately proved by 45 |
| 26–27, 29–30 | Native Qwen/Gemma configure and blocking completion PASS | Four distinct formal invocations; cached MLX/safetensors/nvfp4, API-observed context 131,072 |
| 28 | Selected Qwen local_inference workflow PASS, all 26 checks | One workflow invocation; factorial 3,628,800 and urine count five independently checked; 11 outer requests, ten successes, one recovered RPC error, three distinct later read IDs |
| 31 | Llama3.1 factorial and urine tasks FAIL | One campaign/two positive task invocations, zero structured or executed tools; decorated call examples correctly remain text |
| 32 | Reporter Qwen2.5-Coder factorial and urine tasks FAIL | One campaign/two positive tasks; each executes a recovered L1 TODO but no target script/read action, then simulates tool results |
| 33 | Reporter-model single-shell-call recovery formal test PASS, all 15 checks | Actual recovered-from-text marker; one returned request, no tool execution or full-task reliability credit; fast priority roles unverified |
| 34 | GPT-OSS 20B factorial task FAIL | One positive invocation; fenced checklist/Harmony-like example, zero tools, no script or required factorial result; strict owned cleanup PASS |
| 35 | Ministral 3 8B factorial FAIL | Eight requests: one TODO success, six JavaScript parse errors, one no-progress block; no final/file/target action |
| 36 | Mistral Small 3.2 24B factorial PASS | Ten outer requests, five successes and five recovered errors; real generic script write and exact N=5/N=20 outputs/final |
| 37 | Granite 4 Tiny-H factorial FAIL | Six TODO requests, zero successes: five missing-content errors and one repetition block; no file or final |
| 38 | Phi 4 Mini factorial FAIL | Prose/fenced examples state correct expected values, but zero tools or file; numbers alone are not execution |
| 39 | LFM2 24B factorial FAIL | HTTP 400 context_length refusal, user-only export and no assistant/actions; observed/trained context 32,768; qualified full owned cleanup PASS |
| 40 | DeepSeek R1 8B factorial FAIL | Inert XML-looking/SDK prose after 19,325 thinking characters; zero tools or script |
| 41 | Command R7B 7B factorial FAIL | Explicit context refusal at trained/observed 8,192; user-only export, zero actions |
| 42 | GLM 4.7 Flash factorial FAIL | Native tools perform partial actions, then SDK-result misuse and a 412-byte escaped-newline self-overwrite; no final or verified required numeric results |
| 43 | Planned Gemma3 4B unsupported-tools negative observed; raw task FAIL retained | Application preflight refusal from no-tools metadata, zero actions; original serving census and cleanup FAIL, separate fresh ten-check closure PASS; no whole-case PASS |
| 44 | Native Qwen3.8 27B Q4_K_M GGUF factorial and urine PASS | Two positive tasks; actual generic script/exact N=5 and N=20 outputs plus unchanged-source read/count five; first illustrative urine quotation reverses source word order |
| 45 | Exact owned isolated-daemon stop and final model-lane closure PASS | Same-start PID 71860 SIGTERM; daemon/listener 11445 and active marker removed, last pair 18834 vacant, native successor 95656 unchanged/empty; lifecycle only |

All 46 planned functional slots have closed outcomes. There are 47 actual attempts: 33 PASS and 14 raw FAIL, including the preserved original Gemma survey failure; the corrected survey is an additional attempt rather than another unique coverage slot. Formal coverage comprises 29 unique model/test cases and 30 attempts (29 PASS, one preserved original FAIL); the workflow adds one PASS. The 16 protocol tasks contribute three PASS and 13 raw FAIL: 12 positive-task failures plus the planned Gemma3 negative observation whose raw task remains FAIL. The negative observation does not add a task or whole-case PASS. Clinical campaigns above, nested calls, assertion labels and lifecycle controls are excluded from this finite-attempt count. No eight-task or all-model green aggregate is constructed.

Corrected surveys report input-plus-cache 39,096 for Gemma and 37,303 for Qwen against unchanged >25,000/valid-weather assertions; the original 21,936-token fixture failed its threshold. Raw corrected timing is end-to-end; the generic historical prefill verdict remains qualified and is not a benchmark. Managed formal cases have 18 captured three-role NI 0 observations and seven gaps across 25 attempts. Short cases can close before external sampling; the corrected survey's 68.8-second final gap is after empty-state closure, with active intervals below 60 seconds. Named lifecycle cases ensure the same selected model twice; different-model replacement is unexercised.

Historical short protocol probes and the reporter's single-shell-call probe establish bounded call handling, not full-agent script/read success. Sequence 32's raw 13 assistant records/empty-six metric consists of one recovered request, six text outputs and six inline checklist notifications per task, with five automated reminders. It does not prove six empty provider replies. Sequence 31 and 34 intended whole-reply rejection and sequence 32 result-tag nonexecution reveal model-task failures without a demonstrated parser regression. Observed serving contexts (32,768 for the reporter model; 131,072 for 31/34) do not prove prompt completeness or truncation.

Owned model unloads/process/sidecar checks pass at their recorded closures except for 43's preserved original failure below; its fresh closure is separate. Startup 25 is not daemon-stop proof. Historical external native PID 73104 continuity fails after 31; successor 95656 is a separately observed baseline. Sequence 31's resource-false metadata is corrected by an immutable supplement because unchanged validation returns True rather than None, preserving the original receipt and genuine failures. Sequence 32 checked the wrong preflight port and lacks actual startup listener ownership capture; saved 18893 vacancy is post-cleanup only. Sequence 34 captures its actual 18783 listener. Initial external sampling gaps of 86.14 seconds (28), 27.65 (31), 20.55 (32) and 14.73 (34) remain qualified. Daemon NI 0 belongs to startup 25 only; sampled healthy resources are not continuous guard or zero-swap claims. Recorded file scope is not OS confinement, and no installed Electron or full-wire proof is inferred.

Sequences 35–38 retain raw requested-versus-successful-tool distinctions and their recovered errors. The independent Granite export has 16 records; the owner's 17-record typo is qualified without rewriting the raw record. Mistral Small's observed context 65,536 is serving evidence only, with no automatic-adaptation inference. Its script differs from the successful write argument by one terminal newline; the actual model tool outputs prove execution, with no reviewer execution. Initial external sampling gaps are 14.93 seconds (35), 13.52 (36), 13.34 (37) and 19.73 (38). Their same-owned-identity model unload/process/listener closures pass and campaign/daemon NI 0 is captured; a separate provider-runner priority is not inferred.

Sequence 39's short failure retains the passive caller's late same-identity assertion failure. No raw compared identity or loop role was persisted, so no mismatch, startup listener, NI or external observer is proved. Exact original and fresh cleanup pass separately; the saved actual/preflight port is 18597 and post-close vacancy is recorded. Three internal RAM/swap/serving samples and boundary disk/thermal observations remain, with no external interval-observer credit. Application token estimates are not independently measured wire counts; the HTTP error and trained/observed 32,768 context do not prove prompt completeness or silently lost tokens. Closed classification and root scope review accept 39 as an explicit context refusal, not silent truncation or a recovery regression. Session-send/attach has an existing P2 inline-notice observer limitation, byte-identical to the delivery base: this case records only `[finished] stop` while the provider refusal is logged. The inline notice is yielded without being appended to stored conversation rows in this branch; the user-only export is consistent with a transient notice and does not prove emission loss. Normal interactive CLI/desktop notice handling is supported only by static source evidence; no raw SSE or actual render witness is captured for 39. The session-send/attach diagnostic gap and rc0 finish ambiguity remain a separate follow-up. This known observer limitation remains outside the provider change; no source fix or retry is made. The authorization of unchanged 40–45 supplies no result credit; only their closed case evidence adds outcomes below.

Sequences 40 and 41 have no eligible model call or recorded target action. DeepSeek's XML-like prose and fictional SDK examples remain inert. Command R's explicit context refusal leaves only the initial user row and the same existing session-observer notice/rc0 ambiguity as 39; normal CLI/desktop handling is static evidence only, with no raw SSE/render proof. Its external observer starts after cleanup and records only CAMPAIGN_ABSENT_END; the returned terminal object was lost, so no exit-0 or runtime-monitoring credit is assigned. Application token estimates are not wire counts. Same-owned cleanup, startup listener and campaign/daemon NI 0 are independently captured for 40 and 41.

GLM's full session contains 29 outer request/response pairs (15 successes, 14 errors) and 40 nested effects (38 OK, two errors). The reduced 21-entry trace export has 13 nested entries and eight direct TODO events, omitting 27 nested effects; the superseded explanatory count typo remains preserved. The correct initial script, successful write/chmod/N=5/N=20 status and output byte counts nine/26 survive, but the original numeric payload bytes do not. After misusing a string result as .stdout/.stderr, the model overwrites the script with literal escaped newlines, leaving one 412-byte shebang/comment with an empty static AST body. Later runs exit zero with empty output; neither that status nor completed TODO satisfies the numeric/final predicates. Actual recorded file operations stay within exact WORK; this is not OS confinement. No reviewer re-executes the script. Initial external sampling gaps are 13.54 seconds (40) and 14.83 (42); both retain sampled resources, actual listener/campaign-daemon NI 0 and strict owned cleanup, without continuous guard, separate runner priority or benchmark claims.

Gemma3's catalog capabilities contain completion/vision and no tools. The planned false-toolshim case visibly receives the bounded application preflight error; its assistant row is application error presentation, not a proved model-generated reply. Raw factorial remains FAIL. The original serving census fails with an empty snapshot, and original global cleanup fails because packaged llama-server PID 68416 remains. Its start identity, parent, model and cause are unbound; no signal to that PID is claimed. The startup passive inventory assertion fails without raw inventory, no observer starts, and no startup listener/NI or continuous no-load evidence is credited. Exact terminal supplement 11ab restores the saved tool-object bytes while preserving the superseded serialization artifact and all original result/census/cleanup failures. Fresh ten-check empty-state/source/identity closure passes separately and does not repair the original case. Required final-head CI and durable evidence verification are recorded below.

Sequence 44 closes two required positive tasks on the current source 2ebc/BR ddbf with native cached Qwen GGUF, API-observed context 131,072, and the retained isolated daemon's one-slot setup from startup 25; no direct model-child argv/slot proof was captured. FACTORIAL's seven outer calls and five successful nested actions write a generic argv script and execute N=5/N=20 with exact integer verification after initial scientific-number rendering. URINE's six outer calls and one nested source read return case-insensitive whole-word count five, with the 659-byte source unchanged; its first illustrative final quotation reverses source word order and retains that excerpt-fidelity qualification. These 13 successful outer calls, six nested effects and 15 extracted traces are inside two tasks, not extra tests. Both complete required finals and raw predicates pass. Actual startup listener 18834 and campaign/daemon NI 0 are captured; a separate runner priority is unproved. The external startup gap is 15.44 seconds, sampled minimum free RAM is 32%, and observed swaps remain recorded. Original and fresh same-owned cleanup pass separately, before the subsequent same-owned stop 45. No new all-model/clinical-campaign green aggregate, continuous resource guard, official origin for this native cached model, per-request wire proof or benchmark is claimed.

Sequence 45's actual exit is zero. The stop receipt compares PID 71860's recorded start identity immediately before SIGTERM, confirms daemon and 11445 listener absence and removes its active marker. The final guarded closure also confirms last pair 18834 vacancy, all owned pair/observer PIDs absent, native successor 95656's identity/listener unchanged with models empty, no sidecars and current source/five/47 bindings. API 11445 is intentionally stopped; no successful API reply is fabricated after closure. The global native daemon is untouched, and historical PID 73104 continuity remains false. Startup 25 and stop 45 are two passing lifecycle controls with zero functional/model-test credit. Final resource observations retain swap and are boundary evidence, not continuous monitoring; final-head CI and durable evidence verification remain required for delivery.

Owned database checkpoint/export proof:

Maintenance closes 30 distinct owned generated databases: 29 live roots and one offline copy. The first run (`82e6bc10`/`c904f062`) exits 1 after 30 main/WAL snapshot pairs and 29 successful checkpoints; those 29 are never rerun. Only the offline copy's DB/WAL/SHM permissions change 0444→0600 (`d2cf9efc`), then the unchanged `529675dc` helper runs once for that copy and exits 0 (`75fee5ee`). All 30 retain logical-dump equality, integrity OK, closed connections and absent/zero WAL. Permission causation remains an inference because the original OperationalError message is uncaptured. The 31 snapshot operations preserve 62 exclusive before files, with no SHM copies or post-checkpoint physical-byte identity claim. Final read-only closure (`ce0cf06f`/`57d301e7`) passes all 11 checks with 74 PIDs absent, no DB-parent handles, native successor 95656 empty and isolated 11445 gone; audit `441d9a91` verifies the before files and 87 references. Earlier read-only count and marker-reader-FD failures remain preserved; later exact marker unlink `2da247bc` leaves case 43's original cleanup FAIL unchanged. Both checkpoint runtime NI observations are 20.

### Archive destination and completion proof

Expected destination: `/Users/wgu/biorouter-runs/local-inference-resume-2026-10-11/`.
Expected file manifest: `/Users/wgu/biorouter-runs/local-inference-resume-2026-10-11/manifest-sha256.json`.
Expected completion marker: `/Users/wgu/biorouter-runs/local-inference-resume-2026-10-11/.local-inference-evidence-owner.json`.
These fixed paths describe the required proof locations; they do not assert archive existence or success.

Order: finalize the reports from actual closed roster/database results as a separate seventh
documentation commit, reconcile and normally push the descendant branch, then verify every
required final-head CI check, close all E/live-harness evidence writers, then exclusively
clone the owned evidence and fully verify the saved manifest and all selected bytes.
Only afterward clean the exact owned debug-cache alias/volume once its dependencies are
closed. Final completion requires the actual archive verification and owned cleanup; a
planned path, tiny clone pilot or completion marker alone is insufficient. No report
amendment is needed merely to insert H2's own hash or an archive PASS claim.

Final-head CI status is recorded in [PR #393's current-head checks](https://github.com/BaranziniLab/biorouter/pull/393/checks);
root must verify its actual required checks before ready. Historical investigations below
retain their original outcomes, failures and limitations.

- **Branch:** `claude/ollama-llamacpp-json-bug-6cc4c2`.
- **Exact worktree:** `.claude/worktrees/ollama-llamacpp-json-bug-6cc4c2`.
- **Historical investigation base:** `main` at `30e1f06221dfeb2322af99489722a7860783a2f3`.
- **Retained delivery base:** `474f40087490ff3628d88b59be13a9995ff946e3`.
- Fetch and compare the branch and base at phase boundaries. Do not edit the primary checkout.
- Checkpoint history is preserved; H1 organizes the area commits. All six tested code
  boundaries compile. Local commit 6 changes only the survey fixture; the final reports
  belong in a separate seventh documentation commit, preserving the tested production tree.
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
- The reporter model's historical single-shell-call probe observed TIER1 recovery. Current
  sequence 33 also passes that bounded recovery check, while both full-agent tasks in 32 fail
  despite successful TODO recovery; neither probe establishes script/read task reliability.
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
  is not continuously monitored. See [context window and settings](../../providers/ollama/context-window-and-settings.md)
  for the equal-to-default model-limit provenance caveat.
- Llama startup recovery verifies the request-slot generation before discarding a failed child.
  An old failed startup cannot kill its replacement. A streamed b11429 `Compute error.` discards
  the failed managed backend before releasing its request lease; a late stream error cannot
  discard a replacement. Runtime compute failures return guidance rather than retrying inference.
- Cloud-tag classification is shared by the CLI and provider. Linux NVIDIA VRAM is probed off
  the async worker. Local prompt timestamps use a day boundary to preserve the stable prefix.
- The final privacy guard refuses cloud alternates and discovered remote aliases for a
  reported-Private session before native or remembered OpenAI-compatible inference. It is
  applied and formatted. Matching e9 privacy-generation checks pass all six context/privacy
  regressions, the workspace run and all 21 local gates. At that stage, five fresh executable
  builds and verified copies were promoted as `9f4d3202`. Current `2ebc7e8b` retains four
  Cargo Fresh=true executables and one Fresh=false survey through the source bridge; no new
  full-suite or gate rerun is claimed. Live classifications above retain their own source scope.
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
Sequence 25 actually starts the owned isolated Ollama server with a 4096 startup default
and one parallel slot; closed protocol cases use that isolated lane with their recorded
model contexts. The exact owned stop at 45 is verified above. These settings belong to the
isolated service, without a current external-service continuity claim. A bounded small
CPU-only model probe on
that user's five-slot server now passes automatic 128k-to-64k reduction, explicit 128k
preservation, and exact cleanup/source-preservation checks. This is Apple Silicon mechanism
verification, separate from Intel hardware and the isolated campaign lanes; see the
[resumed validation](local-inference-validation.md) for actual outcomes and retained failures.

## Delivery verification requirements

- Preserve source/binary generations, original failed attempts and separate raw, transport,
  strict-quality, workflow and recorded-scope verdicts. Fix demonstrated product defects
  before rerunning affected checks; ordinary model-quality failures remain disclosed.
- Bind the final roster, actual artifact inspections and any conditional thinking-disabled
  omissions to closed receipts. Formal off-mode coverage must be real before omitting tails.
- Preserve the original research folder and record owned database checkpoints, exact-owned
  cleanup and durable-copy/archive completion. Planned saves or pilots are not final saves.
- Preserve compiled area commits and the survey-only sixth commit. Fetch and compare before
  a separate seventh documentation commit and normal push when the remote remains H1 and the
  local branch descends from it; changed remote state requires fresh reconciliation.
- [PR #393](https://github.com/BaranziniLab/biorouter/pull/393) supplies current delivery and
  [final-head CI status](https://github.com/BaranziniLab/biorouter/pull/393/checks). Actual
  required final-head checks must pass before ready; an H1 pass or launched H2 check is insufficient.

## Known work outside this PR

- Session-send/attach drops inline notices through an existing base-identical observer renderer.
  Sequence 39 logs an explicit context refusal but its closed send output shows only stop.
  Inline notice is yielded without storage in this branch, so the user-only export does not
  prove emission loss. Normal CLI/desktop handling has static source evidence only, with no
  captured SSE or runtime/render proof. The P2 observer diagnostic gap and rc0 finish ambiguity
  remain a separate follow-up outside this provider change.

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

- [Ollama provider](../../providers/ollama/README.md), including the
  [context window and settings](../../providers/ollama/context-window-and-settings.md) and the
  [tool-call protocols](../../providers/ollama/tool-call-protocols.md).
- [Llama Server](../../providers/llama-server/README.md)
- [Providers index](../../providers/README.md)
