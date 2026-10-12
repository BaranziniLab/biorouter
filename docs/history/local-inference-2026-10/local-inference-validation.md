# Local inference final validation

> **What this is.** The source-scoped validation record, including retained model-quality failures
> and separately required PR/durable-evidence delivery proof.
> **Status:** Historical record — local-inference validation concluded 2026-10-12 UTC (2026-10-11 PDT);
> implementation is in unmerged [PR #393](https://github.com/BaranziniLab/biorouter/pull/393).
> Results retain their tested source scope and model-quality failures. Final-head CI is
> recorded by [the PR's current-head checks](https://github.com/BaranziniLab/biorouter/pull/393/checks).
> The [handoff](local-inference-handoff.md) preserves the original investigation.
> **Audience:** maintainers reviewing provider changes and validation evidence.

## Checkout and scope

- Worktree: `.claude/worktrees/ollama-llamacpp-json-bug-6cc4c2`.
- Branch: `claude/ollama-llamacpp-json-bug-6cc4c2`.
- Historical investigation base: `30e1f06221dfeb2322af99489722a7860783a2f3`.
- Retained delivery base, including the idle-timeout correction: `474f40087490ff3628d88b59be13a9995ff946e3`.
- Preserve the original research folder, `~/biorouter-runs/local-inference-2026-10/`.
- Resumed checks and live runs use separate evidence folders and freshly built binaries.

This report records whole-tree counts, local gates, live outcomes, source/binary identities
and compiled area boundaries. Final-head CI status is recorded in
[PR #393's current-head checks](https://github.com/BaranziniLab/biorouter/pull/393/checks).
No planned, ignored or unrun model task earns a live pass.

Late code review found that a cached Ollama window could ignore a later explicit numeric
limit passed to the same provider instance, or let an explicit first call populate the
automatic cache. The correction and three request-body regressions are applied and formatted.
Local verification of that correction completed against source identity
`97554ff6b74dc8d9f891146efa6f690f7b55ba988ed9e5362032ff6dfb66b0d8`.
That revision is historical: the later privacy correction has matching workspace, gate
and boundary passes. Each live result below retains its own source identity; earlier
checks and campaigns are not credited to the later revision.

## Privacy-generation code verification

### Survey-fixture-only current generation

The current local source is commit `dfd0db50022be648303e79d174132616376ef20b`, identity `2ebc7e8b990e161f236e8fd1a08cbeb38dc14d6f24f01b7d4496b7e22c65cd3a`. Only the survey repeat count changes from 560 to 1,000 with truthful end-to-end labeling; acceptance predicates and production source are unchanged. The production-except-survey equality is `77bed48e`, witnessed by bridge `250c5e8e`. Actual promotion `ccfb77e8` binds current receipt `ddbf3422`: four unchanged executables are Cargo Fresh=true and the survey is Fresh=false (`ac42d6f6`). The complete prior `9f4d3202` five-copy generation and its raw receipt are archived. The previous e9 full-suite/gate checks retain their tested scope; this bridge does not claim a new full production-suite rerun. Current aggregate is `d3541e51`, with control `e217e8e8`, observer `e2929121` and historical fixtures unchanged.

Final review found that a Private Ollama session could dispatch a cloud alternate after
remembering an OpenAI-compatible fallback, bypassing the native request's refusal. The
applied guard checks the selected model before either transport and also checks discovered
remote aliases. Three new regressions cover remembered fallback, every request kind,
native alias discovery and the permitted Public-cloud control.

The formatted provider hash is
`00b64dd825210b93581423d5367ce921dc0884afc91f37ebb84c577dfaebc7e9`.
The prepared source candidate was `e30868735cd15dae8a9ffdafa2773c8606919b6c`, with expanded source identity
`e9b21f6fffbc669451a3ef468756f0cf81d2da82fb7296da1a59b2a287e3cac9`.
The source snapshot is saved, and all six matching commit-boundary checks pass locked,
offline, whole-workspace/all-target Cargo checks with one job. All four required desktop
typecheck invocations and three landing JavaScript syntax checks exit zero. The temporary
checkout was removed with exit zero; the task branch and index, including raw index bytes,
stayed unchanged. The receipt is `candidate-boundaries-an92hyte/receipt.json` (`77800381`).
The matching e9 workspace library/binary run exits zero with 17 result summaries: 9,328 passes,
zero failures and 11 ignored tests. The core library has 4,951 passes and three ignored.
Verified provider subsets are Ollama 55, formats 322, Llama Server 34, sidecar 54 and toolshim 13.
The receipt is `late-override-validation-20261011T185146Z-355a2b19/workspace-final.receipt.json`
(`5e49872e`). Six context/privacy regressions, 55 Ollama tests, 46 affected core integration
tests, 25 local-route/server integration tests and seven privacy tests also pass. These
counts overlap the workspace and provider subsets and must not be summed.

All 21 matching e9 local gates pass, and their process exits zero. The canonical checks pass
with the unmodified two-job configuration; OpenAPI regeneration/drift and formatting pass.
The CLI, daemon and three live-test executables were freshly compiled, each with Cargo's
`fresh:false` record, and all five saved copies match their recorded source hashes.
The three default live-test guard invocations pass 11 non-model tests and ignore 15 model
tests; ignored functions earn no model credit. Explicit formal outcomes belong to the final
roster below. The validation receipt is
`late-override-validation-20261011T185146Z-355a2b19/validation-receipt.json` (`0a71a13a`),
and its binary receipt is `binary_receipt.json` (`31c007ac`).

Source, HEAD, staged content and flags remain unchanged. Raw index bytes changed through
stat metadata; that qualification is explicit, with no restoration or test rerun.

At that stage, all five validated executables were promoted into the live harness by a same-volume
atomic rename, without recopying from the mutable build cache. That stage's copied-binary
receipt is `binary_receipt.json` (`9f4d3202`), and the relocation receipt is
`late-binary-promotion-lfs998qz/relocation-receipt.json` (`f642c533`). The previous `04257524`
receipt and five executables are preserved in the live harness's
`bin-archive/042575-20261011T194922Z-cc22274444cf4300ac7ff7e1a7e3e89f/`;
the archive receipt is `old-five-bin-archive-20261011T194922Z-4abfc61f/archive-receipt.json`
(`55b5b2bb`).

That harness preparation was rebound to control `e217e8e8`, observer `e2929121` and
aggregate `375a3ab7`. Its aggregate application receipt is `73967d78`; the prior privacy
aggregate `0d0869ba` remains archived. The seven focused timeout checks (`d28a4a21`) pass
separately from the unchanged 332 historical synthetic fixtures. Preparation adds no live cases.

## Current managed Gemma outcomes

The original G attempt (`97a119a0`) requested eight tasks, invoked four and model-completed
three. Factorial and note counting pass raw and strict review. LOAD fails grounding/schema/
final-claim quality. QUERY is interrupted by the disk guard without a final; its partial
delete-tool name proves neither execution nor mutation. Four tasks were not run. All 14
owned processes closed and serving state emptied, but the original resource closure fails
only the disk floor. Its nonzero campaign/observer exits and all failures remain preserved.

The distinct R6 attempt requests the remaining six tasks and attempts two; both fail and
CURATE, LABS, DEBUG and VIZ are not run in that attempt. LOAD (`6c8c8338`) exits zero with
real tool calls, exact raw-source staging, two pages, reciprocal text references and a
complete final. Its raw and strict checks fail: curated pages omit the fictional qualifier
and required clinical mechanism/cohort/dose/efficacy/safety/biomarker facts, and use invalid
BioOKF types, identifiers/provenance/layout and incomplete schema/module workflow. This
case fails omission rather than inventing intrahepatic-only enrollment. Requested stub
bodies match saved content; no provider/writer content loss is observed. Narrow transport
and artifact successes do not establish clinical quality or task completion.

QUERY (`f80bfacc`) reaches the fixed harness deadline (rc124, no resource-guard kill) after
one completed todo tool and no final or KB retrieval. The sealed daemon warning proves a
KB-delete request was routed through the user. Complete arguments, an approval/cancellation
and a delete result are unavailable: misdirected gated intent is observed, execution and
its terminal gate state are unproven. The timeout is not a demonstrated provider transport
failure or approval-subsystem defect. Only the completed todo has verified recorded scope;
the full pending turn cannot be assessed. A later closed file-only comparison (`3c7a90c5`)
finds all 76 original after-LOAD snapshot files unchanged, with no extras/errors. This is
selected-file preservation, not OS confinement or a complete pending-request export.

R6 cleanup passes separately (`6c8e3e01`): nine owned processes are absent, models/sidecars
empty, native service and source/five executables/47 preparation bindings unchanged, and
the private port vacant. Actual terminal exits (`38a4ce30`) are campaign 1, observer 0,
file-watcher 1 and closure 0. Cleanup success does not turn this incomplete campaign green.
The original G disk-floor failure remains separate; its old disk-blocked status is not
asserted for R6. Earlier exact-owned cache pruning adds no live credit.

| Task | Original G attempt | R6 attempt |
| --- | --- | --- |
| Factorial | Raw/strict pass | Not requested |
| Note counting | Raw/strict pass | Not requested |
| Knowledge load | Grounding/schema/final-claim fail | Raw/strict fail despite correct transport/persisted stub bytes |
| Knowledge query | Disk-interrupted, no final | Deadline timeout, no final; gated delete intent, execution unproven |
| Knowledge curation | Not run | Not run |
| Lab summary | Not run | Not run |
| Debugging | Not run | Not run |
| Visualization | Not run | Not run |

These two attempts are not merged into an eight-task pass. Later independently seeded
CURATE and separate LABS/DEBUG/VIZ attempts retain their own receipts in the final roster.

## Remote CI at interim H1

Interim PR commit `8be80b43` passes all 16 expected checks. Actions tested merge `7f40459d` into main `30e1f062`. macOS workspace: 9,328 passed, 11 ignored; Ubuntu: 9,319 passed, 11 ignored; Windows retry: 8,868 passed, 7 ignored. Vitest: 10,408 passed, 19 skipped across 747 files. These platform counts overlap and are not summed as unique tests. The first Windows Crew contention failure remains preserved; the same test passes on one unchanged-head retry, without proving its original cause. Windows separately passes 17 sandbox tests with one ignored; its no-network integration, production socket guard and host Clippy steps are configured skips. Successful jobs carried into Rust attempt 2 were not rerun. Final-head CI is recorded by [PR #393's current-head checks](https://github.com/BaranziniLab/biorouter/pull/393/checks).

The separate macOS sandbox phase passes 18 with one ignored. Ubuntu separately passes 18
sandbox tests with one ignored and 1,220 no-network integration tests with 65 ignored and
five filtered. Those phases are not summed. Vitest's 19 skips include 13 search-highlighter,
two CDN-asset and three artifact-security browser cases plus one updater case. This is
not installed Electron or live-model evidence. H1 closure (`bca91098`) and its actual
Windows retry (`61733d42`) preserve the original failed attempt and configured skips.
H1 does not replace actual final-head checks before the PR is marked ready.

## Final roster and durable evidence

The planned final roster includes the independently seeded CURATE case, separate LABS/DEBUG/VIZ
cases and the fixed formal/protocol/reporter/workflow campaign: 43 functional launches containing
46 invocations, plus two lifecycle controls. Closed coverage through 39 is recorded below. Source scopes,
quality versus transport verdicts, conditional omissions and artifact inspections must
come from closed receipts. The original after-LOAD snapshot (`9cc3ed77`) remains immutable;
a relocated CURATE seed requires two absolute registry runtime paths to be rebased. Its
actual seed receipt must distinguish 75 byte-exact files from the one rewritten registry,
rather than claim 76 byte-exact runtime copies.

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

Owned database checkpoint/export results:

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
historical H1 results retain their tested scope. The PR's required final-head checks must
actually close successfully before ready. No future hash or planned check is credited.

## Verification of the subsequent context-override correction

The new frozen source's format check, all three exact request-body regressions and the
52-test Ollama provider suite pass. Four affected core integration suites pass all 46 tests,
including the real-capture protocol replays and streaming recovery guards; three server
integration suites pass all 25. The whole-workspace library/binary run passes 9,325 tests
with zero failures and 11 ignored tests across 17 binaries.

The subsequent CI-equivalent recipe stopped when its resource guard observed less than
10 GiB free disk. Its exact owned process group was stopped and reaped; the incomplete
recipe is not a pass. Completed gate results and the original failure are preserved.
After the closed attempt, only this worktree's reproducible debug cache was reclaimed.
The resumed `just check-everything` recipe then returned zero and completed every check,
including strict and baseline Clippy, the socket-inheritance guard, UI lint and all 884
contrast assertions, OpenAPI, naming/version checks and the registry checks. Its wrapper
stopped because it could not establish that its Cargo shim reduced the scripts' explicit
two-worker Clippy request. This is a worker-provenance failure, preserved separately from
the successful recipe result; it is not claimed as a one-worker run. A direct one-worker
Clippy check now passes with explicit `--jobs 1`. The CLI, daemon and all three live-test
executables were freshly built from this source; all eleven non-model guards pass, with
fifteen model tests reserved for explicit live runs. Their exact saved copies were hashed
and moved into the harness without rebuilding or copying from a mutable cache. The final
wrapper stopped after all checks because Git index metadata changed. A closed-run check
confirmed unchanged source, HEAD, all 4,551 staged entries and flags, and revalidated all
eighteen successful gate results and five binaries. The original wrapper failure is retained;
raw index-byte equality is explicitly false.

All six new commit boundaries pass: the
retained base and five area commits each pass locked, offline, whole-workspace/all-target
Cargo checks; desktop checks pass at the affected boundaries, and landing JavaScript parses.
The temporary checkout is removed, and the original branch and index stayed unchanged
throughout those six checks. The build
cache now uses a fresh APFS image on the external drive; source and Git paths are unchanged.
Historical frontend and unaffected integration results retain their original scope below.

## Verification completed before the live-discovered schema follow-up

| Check | Result |
| --- | --- |
| Earlier whole-workspace unit tests | 9,313 passed, 0 failed, 11 ignored, across 17 binaries |
| Selected Rust integration suites | 1,191 passed, 0 failed, 64 ignored, across 167 targets; sandbox exceptions below |
| Platform sandbox integration | 18 passed, 0 failed, 1 ignored |
| Final targeted Ollama provider tests | 49 passed, 0 failed, including two stalled-response-body regressions |
| Crew transport suite and repeated natural-exit regression | 30 passed; the regression passed 20 consecutive runs |
| Historical `d06` full desktop Vitest suite | 10,408 passed, 19 skipped, across 747 files |
| Desktop packaging and console-census scripts | 47 passed |
| Crew manual regression suite | 53 passed |
| BAAM real-browser privacy suite | 16 passed |
| Example-app build and executing browser smoke | All 6 passed, including the ignored smoke test |
| Installed-Chrome browser gap checks | All 18 passed, 0 skipped; CSP, search geometry and CDN artifact handling |
| Generated website documentation | All 14 pages match their tracked sources |
| CI documentation placement rules | 0 findings in the four enforced rules; other rule backlog is excluded |
| Proposed commit boundaries | All 6 passed; whole-workspace/all-target checks and desktop checks at affected boundaries |

The unit run and `just check-everything` passed after the metadata probe follow-up.
The complete frontend suite and example-app browser smoke pass. All 18 browser assertions
skipped for the absent Playwright runtime pass separately with installed Chrome 155. One
Windows-only updater test remains intentionally skipped on macOS and is not covered by the
Ubuntu-only frontend CI job. The final CLI, daemon and three live-test binaries were rebuilt and hashed against the frozen
source identity. All six proposed commit boundaries compile; the original branch/index are
unchanged and the temporary checkout was removed.
The initial live diagnostic exposed a pre-existing tool-schema discovery gap. The renderer
and live-suite assertions were corrected; subsequent local validation is recorded below.
The earlier six boundary checks apply to the pre-schema snapshot. The current boundary
recheck is recorded below; this historical checkpoint does not establish later live completion.

## Verification after the schema and fixture-parser follow-ups

The frozen source identity is
`d06aaf0f6ed9f44256771f9854bf606a4136033416b42e3f3168687781fdcfde`.
The inventory includes externally embedded Rust inputs. A byte-identity bridge to the original
`6adb6afb320483c17b761f0db571940546d27b5a61ccae45fb3a35013b9476ec` inventory
proves that the candidate and all compiled inputs are unchanged; only the inventory expanded.
The 2026-10-11 whole-workspace library/binary run passed 9,322 tests with zero failures and
11 ignored tests across 17 binaries. The focused namespace parser module passed all 13 tests,
the tool-signature renderer passed all 114 tests, and the ignored live suites' non-model
guards passed all 11 tests. Fifteen actual model tests remain ignored in that default run;
they require separate live invocations.

The refreshed integration run completed all 167 selected targets. After the three unchanged
filesystem/process fixtures passed outside the interfering policy (7, 8 and 9 tests), the
verified aggregate is 1,202 passed, zero unresolved failures and 64 ignored tests. The original
command's exit 101 and failure logs are preserved; the exceptions are described below.

The post-schema `just check-everything` rerun passed. Example-app checks, including the
executing browser smoke, passed all six tests. OpenAPI regeneration produced no drift,
Rust formatting passed, and the CLI, daemon and three live-test executables were rebuilt.
Their hashes and frozen source identity are recorded in the refreshed binary receipt.

The desktop source is unchanged from its historical `d06` 10,408-pass Vitest suite and separate
installed-Chrome checks; those results are retained at their original scope.

All six post-schema commit boundaries passed locked, offline, whole-workspace/all-target Cargo
checks. Desktop checks passed at the retained base and affected commit boundaries. The inventory includes
all 16 externally embedded Rust inputs, including the documentation commit's changed inputs.
The temporary checkout was removed and the original branch, HEAD and index stayed unchanged.
That local gate aggregate is green only for the stated `d06` source. Later code, live and
delivery evidence is recorded above at its own scope; no later result is credited to this
historical checkpoint and no planned model task earns a pass.

## Initial failures and their resolution

The first resumed full unit run encountered one pre-existing Crew transport assertion that
expected only broken pipe after a child exited. The actual safe result was SSH EOF with exit
status 23. The test now accepts those two terminal paths while retaining exit status, unusable
transport and secret-redaction assertions. Production transport is unchanged. The full unit
rerun passed, and the focused regression passed 20 consecutive times.

The first `check-everything` stopped at Clippy's type-complexity warning for the new shared
memory-cache declaration. Named key/map types resolve it without disabling the lint. Final
review also found that `/api/show` and `/api/version` bounded response headers but not the JSON
body. Both probes now bound the complete response to 15 seconds. Tests send headers followed
by a stalled body, then verify a successful uncached retry and cached reread.

After Clippy passed, the socket-inheritance census could not complete with the offline cache
because a cross-platform dependency was absent. `cargo fetch --locked` populated those pinned
entries without changing `Cargo.lock`; the census then passed offline for all 17 production
targets. TypeScript caught an unsupported `Array.at` call in the new gauge regression; its
assertion now uses the existing JavaScript target, and UI lint and format checks pass.

The initial network-sandboxed integration command exited 101. Three unchanged baseline
fixtures were rerun outside that policy after verifying its interference:

- `daemon_runtime_contract`: the policy stripped set-ID permission bits after a successful
  `chmod`. A direct mode probe confirmed the difference. All seven original tests pass outside
  that policy.
- `apps_serve_lifecycle` and `serve_lifecycle`: process inspection failed because the policy
  denied launching `ps`. All eight and nine original tests respectively pass outside it.

The test sources and runtime source match the base commit. The corrected aggregate retains
all 167 selected targets and the raw failure log; no privacy assertion or permission fixture
was weakened. Only an exact identified test process was stopped during cleanup.

The schema-follow-up unit run later reported seven failures: six unchanged Bedrock namespace
fixtures rejected valid child observations that serial libtest printed after its test-name
prefix, and one unchanged Crew SSH-alias fixture used the hostname fallback. The parser now
accepts only a standalone observation or the exact expected libtest prefix, rejects duplicate
observations, and requires successful grandchild exit. All original privacy assertions remain.
The 13 namespace tests pass, including three new parser regressions.

The original Crew alias test passed ten isolated sequential runs at nice 19 and passed again
in the current whole-workspace rerun. Isolated libtest durations were 0.22–0.42 seconds; the
earlier failure took 6.01 seconds across a five-second lookup deadline. Load-induced fallback
is an inference; production deadlines and Crew fixtures are unchanged. The failed run and
successful diagnostics are preserved; the refreshed whole-workspace unit run passed with
zero failures.

## Live verification

On the earlier `97554ff6` source, Gemma/Ollama with an explicit `OLLAMA_THINK=false`
completed factorial, lab analysis and debugging. All three pass independent correctness,
recorded action scope, fixture preservation and complete-response checks. The review covers
87 saved messages, 38 outer requests and 20 nested calls. Seven tool errors were recovered,
including the initial failing debugging tests; the final unsuppressed run passes all eight
unchanged tests. All five lab-summary rows match an independent decimal calculation.
No thinking characters were observed. This verifies the provider-default configuration,
not a captured wire-level `think` field or a causal performance comparison.
Strict cleanup and a fresh post-exit check prove all known owned processes absent, empty
model/sidecar state and unchanged source and five binaries. The resource observer exited
normally; its minimums were 42% free memory and 13.999 GiB free disk.

The managed Qwen/Llama Server default pair also completed all eight tasks on the earlier
`97554ff6` source and its `04257524` copied-binary receipt. Seven pass the raw artifact
checks, four pass strict independent quality/workflow review, all eight pass recorded
task-file path scope, and seven have substantive final answers. Overall quality is false.

| Managed Qwen default case | Raw artifact check | Independent quality/workflow |
| --- | --- | --- |
| Factorial | Pass | Pass: correct 5! and exact 20! |
| Note counting | Pass | Pass: five whole-word matches, source preserved |
| Knowledge load | Pass | Fail: unsupported intrahepatic enrollment and incomplete module inspection |
| Knowledge query | Pass | Fail: required module inspection omitted; numeric answer and fictional qualifier are correct |
| Knowledge curation | Fail | Fail: invalid YAML, unsupported phase 3 randomization and no final answer at 40 turns |
| Lab script | Pass | Pass: all five rows and P002/P005 counts correct |
| Debugging | Pass | Pass: unchanged eight-test fixture, actual unsuppressed final eight tests OK |
| Visualisation | Pass | Fail: correct means and artifact, but an unspecified mg/dL unit was added |

Loading preserves the supplied source, core numbers, fictional labels, typed pages and
links. It incorrectly narrows the trial cohort to intrahepatic disease; the source's separate
prevalence statement does not support that qualifier. Querying correctly cites the study
and reports its numbers without repeating the bad cohort claim, but skips the required
`read_module` workflow. Curation preserves phase 2 facts and saves the new numeric facts,
then leaves a new YAML error and an unsupported randomized-trial qualifier. Actual lint
reports one error and four warnings; no complete final answer follows the unchanged action
limit. Visualisation's five means agree with the CSV and returned HTML; the CSV and task
did not supply the added mg/dL unit, which remains a quality failure. Actual standalone
Chrome inspection verifies all five bars and pointer tooltips (0.862, 1.376, 1.122, 0.706,
1.294), the matching data table, and its expand/collapse control. No console warnings or
errors were captured. Seven JPEGs and expanded/collapsed accessibility snapshots are
bound by `chart-visual-762e31-chrome-20261011/actual-visual-receipt.json` (`b864a468`).
The owned browser tab and loopback viewer closed, and the viewer exited zero. IAB was
unavailable; this covers standalone Chrome, without Electron, IAB or HAR verification.

The closed-case audit binds 251 saved session rows, 117 outer requests with eight errors,
and 89 nested calls with six errors. Twenty-one nested argument strings are truncated;
complete outer requests and immutable raw/page snapshots qualify those observations.
The CLI campaign exits nonzero because curation fails; the observer exits normally.
Fresh qualified cleanup passes all 16 ownership/source/binary checks, with no serving model
or owned process left. Its original checker failure is retained: it compared the native
model-response schema and observer's extra argv too strictly. The later qualification
does not rerun or turn the failed model case into a pass. Recorded task-file scope is not
OS confinement, and these cold/cache/shared-load timings are not benchmarks.

Historical `d06` campaign. The first Qwen/Ollama default pair completed all
eight cases: eight semantic/artifact successes, six observed scope-compliant successes and
two scope-quality failures. Its overall quality verdict is a failure. The phase 2 ingestion
contains the exact source text, both required pages and two reciprocal links; actual lint
reports zero errors and warnings. Curation adds the supplied phase 3 results. The native 131,072-token context
served a 31,159-token bootstrap recorded in persisted provider usage. This counter is not
a raw wire capture. Debugging passes all eight unchanged tests on independent rerun.
The chart renders five correct bars and an accessible table with all five expected values
in the Codex in-app browser. This standalone HTML check does not establish packaged Electron
preview behavior. Exact owned-process cleanup, model unload and empty serving state pass.
The scope failures below remain part of the result. Functional wall times include cold
and cached work and are not benchmarks.

| Qwen/Ollama default case | Seconds | Semantic/artifact check | Observed task scope |
| --- | ---: | --- | --- |
| Factorial | 308.30 | Pass | Pass |
| Note counting | 151.94 | Pass | Pass |
| Knowledge ingestion | 480.00 | Pass | Pass |
| Knowledge retrieval | 206.96 | Pass | Pass |
| Knowledge curation | 988.51 | Pass | Fail |
| Lab script | 416.65 | Pass | Fail |
| Debugging | 390.93 | Pass | Pass |
| Visualization | 243.54 | Pass | Pass |

Scope checks review recorded requests and responses; they are not syscall tracing or proof
of an OS/path sandbox. The immutable pair-quality supplement retains both failures, the
model's false final curation claim of complete scope compliance, all case bindings and
cleanup evidence. A later root receipt records the completed chart visual review.

The curation task attempted a filesystem-wide directory search despite the fixture scope
instruction. The saved tool response reports its 240-second timeout and process-group
termination. This is a model scope/quality failure even if subsequent artifacts pass.
No file or credential content read was observed in that command's saved output. Its private
receipt is retained separately; artifact correctness does not erase the scope failure.
The model returned to the offered knowledge API. Its final curation artifacts preserve the
phase 2 results, include the supplied phase 3 results, retain reciprocal links and pass an
independent lint. The final scope-quality supplement still records a failure. The fresh
thinking-disabled retest with explicit working and profile directories below also fails
completion and quality.

The lab-script task redirected its own debugging output to `/tmp/analyze_labs.log`, then
requested an editor read outside the supplied working directory. The editor rejected that
read. This is a separate fixture-scope failure; no unrelated file or credential read was
observed. The model returned to the working directory, fixed its script, and passed an
independent rerun and all five expected output rows. This semantic recovery does not erase
the scope finding. Its fresh clarified-fixture retest below passes; the original failure remains.

A separate default-thinking rerun uses explicit working/profile directories and directs
knowledge tasks to the offered APIs. Factorial passes in 276.25 seconds, with all nine
recorded requests independently reviewed as scope-compliant. Knowledge loading times out
at its unchanged 1,800-second deadline (1,801.99 seconds measured): its raw source and six
pages were written, and the limited artifact checks pass, but no final transaction-commit
call, final lint or final answer is saved. The task fails; curation, lab scripting and
debugging were not run in this pair. Its recorded requests stay within the clarified scope.
The model selected BioOKF rather than the original pair's OKF format; the task does not
require either format. The recorded 136,991 thinking characters and wall time are functional
observations under host load, not a performance benchmark.

The timeout's inner owned-process cleanup succeeds. The campaign then attempts a second
stop, which fails closed because the first cleanup removed its PID file. The raw pair
summary retains that failed cleanup gate. A separate read-only observation confirms that
all six known owned PIDs are absent, no model or sidecar remains, and available memory has
recovered to 73%. The harness now validates and reuses the exact inner cleanup receipt,
with fresh process/model absence checks, while retaining the task failure. Its 41 targeted
mocked checks pass, and a separate read-only reconciliation of the real timeout cleanup
passes. The original summary remains unchanged. The fresh thinking-disabled pair is recorded
below; this partial run does not replace either original scope failure.

A fresh five-task Qwen/Ollama pair disables thinking with the same clarified fixture and
unchanged 40-turn and 1,800-second bounds. All five cases finish: the raw verifier passes four,
independent quality review passes three, and the pair's overall verdict is a failure.
All 100 recorded outer requests and their responses are reviewed. Recorded file actions stay
within the supplied working/profile directories; the curation API-workflow failure remains.
Factorial passes in 257.39 seconds, with 11 outer
calls and one recovered missing-path tool error. All recorded requests stay within the
supplied directories. Its first persisted provider-usage counter records 31,311 fresh input
tokens and zero cached tokens; this is not a raw wire capture of the thinking parameter.

Knowledge loading finishes in 786.43 seconds with 27 outer calls and no tool errors. The
exact supplied raw source, five pages and reciprocal links are present, so the limited
artifact verifier passes. Independent source review fails the semantic grounding check:
the drug and trial pages narrow enrollment to previously treated *intrahepatic*
cholangiocarcinoma, while the source states cholangiocarcinoma for enrollment and mentions
intrahepatic disease only in a separate prevalence sentence. The saved graph has 14 typed
and four plain edges; the model's claim of 17 typed edges is incorrect. Actual final lint
reports zero errors and four warnings, including a non-scholarly-source warning. Loading
does not require repairing every lint warning; curation does. These findings are retained
with an immutable snapshot taken before any curation tool request. Recorded filesystem
scope is compliant; artifact success does not erase the grounding failure.

Curation stops at the unchanged 40-turn bound in 1,068.98 seconds, with 40 outer calls,
three tool errors and no final answer. The phase 3 raw source, trial page, drug update and
reciprocal links are saved. The model does not run the requested full knowledge-base lint
or repair every finding. Independent lint reports zero errors and eight warnings; the
harness's no-errors predicate does not establish warning repair. The case fails. Independent
review also finds unsupported phase 3 randomization and a drug description that incorrectly
applies previously treated, intrahepatic disease to both trials despite the supplied phase 3
first-line source. Recorded file paths stay within the supplied directories, but the model
calls knowledge tools before reading their module and searches/reads the knowledge store
through the filesystem contrary to the offered-API workflow instruction. Returned shell
truncation metadata mentions an external temporary output path; no recorded request reads
that file.

Lab scripting passes in 272.31 seconds with 11 outer calls and no tool errors. The source CSV
is unchanged, all five generated rows agree with independent arithmetic, and P002/P005 are
correctly identified. Debugging passes in 162.05 seconds with 11 outer calls and one expected
failing-baseline response. Its repaired helper passes all eight unchanged tests on actual
and independent harness runs. Both cases pass recorded scope review. Their full saved answers
are complete; the verifier stores bounded excerpts. Exact owned-process cleanup, model unload,
observer exit, empty serving state and all five unchanged binary hashes pass.

| Qwen/Ollama thinking-disabled case | Seconds | Raw verifier | Independent quality |
| --- | ---: | --- | --- |
| Factorial | 257.39 | Pass | Pass |
| Knowledge ingestion | 786.43 | Pass | Fail: grounding |
| Knowledge curation | 1,068.98 | Fail | Fail: completion, grounding, API workflow |
| Lab script | 272.31 | Pass | Pass |
| Debugging | 162.05 | Pass | Pass |

These controls preserve the original outcomes and do not establish an eight-task pass,
native wire capture of `think:false`, or comparative performance.

A separate fresh default-thinking pair completes lab scripting and debugging with the same
clarified prompts and fixtures. Both pass artifact and independent quality review. All 62
closed-session messages, 29 outer requests/responses and 21 recorded nested actions are
reviewed; file actions stay within the supplied working directory. Lab scripting calculates
all five rows from the unchanged source CSV without reading the expected-result fixture.
Debugging retains its failed baseline and intermediate median failure, then passes the actual
verbose eight-test run and independent harness replay with the test file unchanged. Both
full final answers are complete. Owned-process shutdown, observer exit, model unload,
empty serving state and the unchanged frozen source and five binary hashes pass.

| Fresh default-thinking case | Seconds | Calls / errors | Independent quality |
| --- | ---: | --- | --- |
| Lab script | 297.57 | 10 / 0 | Pass |
| Debugging | 293.28 | 19 / 3 | Pass |

These two matched-fixture cases record 2,062 and 6,258 thinking characters respectively.
Their outcomes do not erase the earlier scope or knowledge-task failures, establish an
eight-task pass, or establish a general quality or timing effect from disabling thinking.
The native wire thinking parameter was not captured.

The Gemma/Ollama default-thinking run completed all eight tasks on the historical `d06`
binaries. Six pass the original artifact checks, four pass broader independent quality
review, and all eight pass recorded path/profile scope review. The initial prompt counter
records 30,530 fresh input tokens; this is saved provider usage, not a wire capture.
The later provider correction is not validated by these historical runs.

| Gemma default-thinking case | Seconds | Calls / errors | Raw artifact check | Independent quality |
| --- | ---: | --- | --- | --- |
| Factorial | 314.53 | 8 / 1 | Pass | Pass |
| Note counting | 323.78 | 8 / 1 | Pass | Pass |
| Knowledge load | 904.06 | 25 / 6 | Pass | Fail: qualification and structured format |
| Knowledge query | 569.11 | 15 / 1 | Pass | Fail: fictional qualification |
| Knowledge curation | 862.64 | 30 / 4 | Fail | Fail: qualification and diagnostic claim |
| Lab script | 387.59 | 12 / 1 | Pass | Pass |
| Debugging | 716.52 | 16 / 5 | Pass | Pass |
| Visualisation | 425.09 | 8 / 1 | Fail | Fail: incorrect source mean |

Knowledge load saves the exact fictional source and two reciprocally linked pages with
grounded cohort and efficacy numbers. Both pages omit the fictional-data qualifier and use
types outside the saved BioOKF schema; identifiers, typed provenance and typed relations are
absent. Initial code calls precede schema discovery, so schema-first workflow also fails.
The model did not run knowledge lint, and independent lint diagnostic counts are not claimed.
The immutable post-load snapshot was saved before subsequent curation.

Knowledge query accurately returns 31% response, 7.4 months median PFS and 9% grade 3+
neutropenia. Actual page reads ground the first two values; a raw-source search snippet
grounds the third. The complete 287-character answer names the page and raw source, but
omits the fictional qualifier. The retrieved pages and snippet also lack that qualifier,
so the limitation is inherited from incomplete curation and partial retrieval. Offered-API
workflow passes. One empty search omitted the base identifier and a base-list call returned
selected-profile metadata; target-base-only querying is not claimed. Recorded file/profile
scope stays within the task.

Knowledge curation ingests the exact phase 3 source, updates grounded efficacy and safety
numbers, and adds bidirectional links. Its raw grader fails because the drug page lacks
31% or 7.4 from phase 2. The original post-load drug page already lacked both numbers;
the phase 2 trial numeric body, raw source and drug link remain preserved. The prompt does
not explicitly require moving those trial numbers onto the drug page, and no deletion of
prior numeric facts is demonstrated. The raw failure remains recorded.

Actual model lint repairs six errors plus two informational diagnostics, then one error,
then zero errors, warnings or information diagnostics. Separate verifier CLI output reports
a clean base. The final `Other` type with a `note` is permitted by the saved schema.
Quality limitations remain despite this lint success: the directly read fictional qualifier
is absent from curated pages and the complete 1,104-character final answer; initial calls
precede schema discovery; typed provenance and index/log maintenance remain incomplete.
The biomarker inequality is saved as malformed `$ge 6$`, and the final answer incorrectly
calls the initial eight diagnostics eight errors. All three knowledge-task quality failures
remain explicit.

The lab script uses the unchanged supplied CSV and standard library, writes all five mean,
maximum and high-count rows correctly, and identifies P002 and P005. Independent decimal
recalculation matches the output, and all 26 closed session messages match the saved export.
The complete final answer and recorded file scope pass. The verifier records a successful
script replay, whose raw stdout was not separately retained.

Debugging preserves the original eight-test file. The failed baseline reports five failures
and one error; the final unsuppressed command and independent verifier both report eight
tests, `OK`. The helper repairs empty-mean validation, nonmutating even-length median logic,
sample variance and independent high-value flags. Five recovered tool errors remain recorded.
This result covers the supplied cases; additional empty-median and small-sample cases were
not tested by this live task.

Visualisation supplies and reports P002's mean as 1.296, whereas the unchanged source CSV
gives 1.376. The returned HTML preserves the submitted value; the other four means are
correct. After model unload, a separate browser inspection confirms five bars, a zero-based
axis and the same five values in the table. The numeric error is already present in the
model's request, and no renderer transformation is demonstrated. This inspection covers the
standalone returned HTML, not the packaged desktop app.

The pair retains all 122 outer calls and 20 recovered errors, its two raw failures and four
broader quality failures. Exact owned-process shutdown and model unload pass a fresh cleanup
check. The auxiliary observer recorded 222 resource observations without a runtime guard
failure, then exited with an error while logging normal campaign termination; that exit is
preserved separately and is not presented as a successful observer exit. At the saved
14:43 UTC cleanup checkpoint, all 19 known owned PIDs were absent and model and sidecar
state was empty. These functional results are not timing benchmarks.

The earlier Qwen/Ollama diagnostic baseline passed factorial and note counting. Its
knowledge-load task was intentionally interrupted after confirming that object-union tool
schemas had lost their `kind` discriminator in discovery; raw ingestion had succeeded, pages
were not completed, and five later tasks were not run. This baseline is not a completed pair.
Its sessions, partial files and interruption/cleanup receipts are preserved separately.

The correction preserves structured union variants and escaped string literals in tool
signatures. Live-suite assertions now require valid calls to the offered weather tool for
Paris, count cached prompt input, reject empty/unknown model selections, force the queue's
one-slot fixture and fail the survey after saving any failed checks. All eleven non-model
helper tests and the 114-test renderer module, including six new regressions, pass.

The copied harness removes its temporary 64k override, verifies actual tool responses,
reruns generated code and tests independently, hashes immutable fixtures, and validates saved
chart data and HTML. It runs one serving model at a time. The final roster records actual
owned one-slot Ollama GGUF-lane outcomes separately; scheduling and control preparation earn
no live credit. The user's five-slot setting is preserved.
A bounded CPU-only probe with an existing Granite GGUF passes all 22 runtime checks
and 16 post-exit preservation checks on the user's unchanged five-slot Ollama service.
It passed first on the earlier `d06` source and again on the earlier `97554ff6` source,
using a fresh UUID alias and the new copied CLI and daemon. Independent review of the
second attempt confirms all 22 runtime and 16 preservation checks, including all ten
owned process roles absent, unchanged native service and full source-weight hash.
Two replies return exactly `ready` without tools in the same Biorouter daemon. Its automatic
context gauge changes from 131,072 to 65,536 after the first reply and stays at 65,536 for
the second, with one downgrade warning and one floor advisory. Actual native runner flags
prove five CPU slots, zero GPU layers and two threads, with total context 655,360 initially
and 327,680 afterward. Loaded allocation changes from 10,766,411,365 to 7,813,359,205 bytes.

A fresh daemon with explicit `OLLAMA_NUM_CTX=131072` returns a third `ready`, preserves
131,072 per request across five native slots, and records no cap or floor warnings. Each
phase unloads the exact owned alias before strict daemon cleanup. The final timestamped
proof confirms all ten probe processes are absent, model and sidecar state is empty, the
temporary alias names are removed, and original manifests, shared content, full source-weight
digest and native Ollama identity are preserved. Independent saved-evidence review passes.

This exercises the adaptive CPU-spill mechanism and explicit override on Apple Silicon;
it does not reproduce the reporter's Intel hardware or the 93 GB/125 GB allocations, prove
agent-task quality, or prevent an unexpected first-load allocation failure. Four earlier
actual attempts remain failed in the evidence record. They exposed native model-store and
runner-observer compatibility gaps, a reload-observation race, and cleanup ordering in the
test harness. The corrected harness retains those failures and the rejected cleanup proposal.

## Follow-up on the 14 failed attempts

After delivery at `98fd881b`, two independent reviews read the archived evidence for each of
the 14 raw FAIL attempts (sessions, verdicts, daemon logs, per-request usage logs and
`/api/show` provenance under `~/biorouter-runs/local-inference-resume-2026-10-11/live`).
Where they disagreed, the code and the logs decided. Four attempts (F10, F11, F12, F13)
traced to Biorouter defects, and the reruns found five more in the same Code Execution
reporting path. All are fixed on this branch with tests that fail without the fix. The other
attempts are model limitations or harness observations; three of them (F04/F05, F06, F07)
still led to Biorouter changes, because Biorouter had hidden from the model or the reader
what was going on.

The reruns used fresh debug builds of the commits named below, the same task prompts, a new
owned one-slot Ollama on port 11445 (version 0.40.3; the original campaign ran 0.40.2), one
model at a time, and never-used pair tags. The last code commit is `35954e31`; the
"final head" results below were run on its build. Evidence, binaries and their SHA-256 sums
are in `~/biorouter-runs/pr393-failed-attempt-reruns-2026-10-12/harness/` (`source_head.txt`
maps each round to its commit).

| Attempt | Model and task | Finding | Change | Rerun |
| --- | --- | --- | --- | --- |
| F01 | Gemma 4 31B survey | Harness: the bootstrap fixture was too small for Gemma's tokenizer | `dfd0db50` (earlier) | PASS earlier, 39,096 prompt tokens |
| F02 | llama3.1:8b factorial | Model: Python sent to a JavaScript tool, with invented parameters, written as text | None | Not rerun; no change reaches it |
| F03 | llama3.1:8b urine | Model, at a deliberate boundary: an 85-character preamble that echoes the model's own template, then one valid call. Calls inside prose are neither run nor asked about, because a quote and an intent cannot be told apart and asking once was measured not to help this model. Widening that rule is a security decision for the maintainers | None | Not rerun |
| F04, F05 | qwen2.5-coder:7b factorial, urine | Model, plus a Biorouter gap: after one call, whole replies imitated Biorouter's own tool-result frame and were accepted as answers, so the model was never told that nothing had run | `b672d90b`: one corrective retry for such a reply; a reply that relays a real result is left alone | At `b672d90b` and at the final head the retry fired (2 and 3 times, daemon logs). Both tasks still FAIL: after it the model answered in prose or wrote a JSON-text call |
| F06 | gpt-oss:20b factorial | Model: a malformed Harmony call after a checklist, inside prose. It contradicted a module doc that said Harmony never reaches Biorouter as text | `49f519e9`: doc corrected | Not rerun |
| F07 | ministral-3:8b factorial | Model, plus opaque errors: Python triple quotes in JavaScript, six times, each answered only with "unterminated string literal" | `359f0323` (triple quotes named), `0e38ef20` (a failed call's output reaches the script), `21fd6649` (an unimported tool names its import), `35954e31` (a script that returned nothing says its calls succeeded) | Still FAIL. At `4010c2f9` the file was written and runs, but `shell({command: "run", args: [...]})` failed three times while the script saw only "exited with status 127", never "command not found: run". At `21fd6649` the model kept its triple quotes through three explained errors, then wrote a tool call whose JSON Ollama could not parse (a backslash before a backtick, an invalid escape), which ended the turn with that error |
| F08 | granite4:tiny-h factorial | Model: wrong `todo_write` arguments repeated until the loop guard stopped the turn | None | Not rerun |
| F09 | phi4-mini:3.8b factorial | Model: answered from memory without a tool call | None | Not rerun |
| F10 | lfm2:24b factorial | Biorouter: Biorouter's prompt and tools overflow a 32,768-token window. The turn ended as an ordinary `stop` with the reason neither printed by `session send` nor stored | `f28ca994`: an unrecoverable overflow ends the turn as a stored, typed `context_length` failure; `session send`, `watch` and `attach` print inline notices | Task FAIL as expected, now with the sentence on screen and in the chat: 7.0 s, and 6.5 s at the final head. The chat-title call that followed the refused turn sent the model a second request; see the review of the rerun below (`8e186442`) |
| F11 | deepseek-r1:8b factorial | Biorouter: Ollama lists `tools`, but neither of the model's templates renders them, so the model never saw a tool definition (19,094 prompt tokens where phi4-mini read the same request as 29,997) | `5b879a69`: such a model is refused before it loads, with a sentence naming the template | The turn was refused before the model loaded: 0.16 s, and 0.13 s at the final head; task FAIL by design. Biorouter's chat-title call then loaded the refused model, and that runner is what each pair's cleanup check found; the owned daemon was gone. Corrected by the review of the rerun below and fixed in `8e186442` |
| F12 | command-r7b:7b factorial | Biorouter: as F10, with an 8,192-token window | `f28ca994` | Task FAIL as expected, sentence shown and stored: 5.5 s, and 5.1 s at the final head |
| F13 | glm-4.7-flash factorial | Biorouter: the script's four calls succeeded, then it read `.stdout` off the string `shell` returns. The error named nothing and never said the calls had worked, and the model destroyed its correct file | `359f0323`, `d281bd6b`, `4010c2f9` | **PASS** at `4010c2f9` (402 s) and again at the final head (463 s): 120 and 2432902008176640000. See below |
| F14 | gemma3:4b factorial (planned negative) | Expected refusal. The runner the case's cleanup checks found was this model, loaded by Biorouter's chat-title call after the refusal; the case also expects a loaded model that a correct refusal never loads | `8e186442` (from the review of the rerun) | Not rerun at this point; see the review of the rerun below |

F13 took three rounds, each finding a further defect in how Code Execution reports back:

1. At `b672d90b` the error named the completed calls, so the model kept its correct file. It
   still had no answer: it reported values with `console.log` and a top-level `return`, both
   answered by bare engine errors, and scripts that never passed a value to `record_result`
   came back as a bare `Result: undefined`. The loop guard ended the turn.
2. At `d281bd6b` those three cases name `record_result`. The model then called
   `record_result({ result5, result20 })`, exactly right, and Biorouter reported 20! as
   `2.43290200817664e+18`: the shell printed `2432902008176640000`, which parses as JSON and
   became a double.
3. At `4010c2f9` an integer result a JavaScript number cannot hold stays the tool's text, and
   the task passed every check. It passed again on the final-head build.

Two results stay open for the maintainers. `biorouter session send` still exits 0 when a turn
ends on an error frame; the frame is now printed, but the exit status is a CLI contract
decision. And Biorouter's fixed prompt is about 29,000 to 32,000 tokens, which no model with a
32,768-token window or less can run; that is a product decision about a smaller tool set for
small windows.

## Rerun of the failed attempts and the formal cases (2026-10-12)

After the follow-up above, every failed attempt that was not purely environmental was rerun at
least twice, and all 29 formal live cases were rerun, with the PR's own live harness. The first
round found one more defect in the Code Execution reporting path (below); it is fixed in
`acbefe5a`, and the later rounds and every formal case ran on that build.

How the reruns were made:

- **Builds.** Build A is the pushed head `46d20424` (its code is `35954e31`; the head adds only
  the follow-up's documentation). Build B is `acbefe5a`, which adds only the fix below. Both are
  fresh debug builds of the CLI, the daemon and the three live-test executables, made in a
  separate worktree on branch `pr393-rerun`, with source identities `70aae2fd` (A) and `dd425f04`
  (B) over the campaign's own 22 compile-input paths. Every executable is hashed in the
  harness's `binary_receipt.json`; build A's receipt and executables are archived beside it.
- **Harness.** A copy of the campaign harness (`campaign_pair.py`, `formal_run.py`,
  `isolated_ollama.py`, `run_task.py`, `verify.py`, the fixtures and the task prompts), changed
  only to point at the rerun worktree, its branch and its evidence folder.
- **Serving.** The protocol tasks and the five Ollama formal cases ran against a new owned
  one-slot Ollama 0.40.3 on port 11445 with a 4,096-token default context, as in the original
  campaign; it was stopped by its exact owned identity afterwards. The Llama Server cases ran
  on the managed sidecar (build 11429) with the cached GGUFs. One model was
  loaded at a time, and the user's own Ollama on 11434 was only observed (empty), never used.
  No remote provider was involved.
- **Evidence.** `~/biorouter-runs/local-inference-resume-2026-10-11-delivery-rerun/`: the
  harness copy with `runs/`, `summaries/`, `daemons/` (each daemon's log), `formal/`,
  `analysis/` (`protocol-analysis.json`, `formal-analysis.json`) and `../logs/`. Protocol tags
  are `pr393rerun-r1-20261012-protocol-NN` (round 1, build A) and `pr393rerun-r2-…`/`r3-…`
  (build B); formal tags are `pr393rerun-20261012-…`. Every tag is new.

### The defect the first round found (F07)

In round 1, ministral-3:8b wrote a correct `factorial.py` and ran it for N=5 and N=20; both
`shell` calls returned their output (9 and 26 bytes). Its script then recorded
`{ outputN5: result5.output, outputN20: result20.output }`. `shell` hands a script its output as
one string, so both properties were `undefined`, `record_result` left them out, and the model
was shown `Result: {}` ten times with nothing saying why, until the task reached its
1,800-second limit. The F13 fix explains a read like this only when it throws (`.stdout.trim()`);
read and recorded directly, nothing throws.

`acbefe5a` carries the same explanation out of a script that runs to its end: the result now says
the calls completed, names each property that was undefined because the tool returned a string,
and shows how to read the string itself. A script that records the string, or whose tool
returned a JSON object, is shown only its result. Two new tests (the verbatim ministral script
through the engine, and a recorded property of a string result through the real handler) fail
without the change; all 138 Code Execution library tests, the 42 `code_execution_integration`
tests, `cargo clippy -p biorouter --all-targets -D warnings` and `cargo fmt --check` pass, and
every CI check on `acbefe5a` passes (cross-build-nightly is skipped by design). In rounds 2 and 3
ministral did not repeat that script, so the fix is covered by those tests, not by a live run.

The review of this rerun found the note false in two shapes and its premise false for numeric
output: "`shell` hands a script its output as one string" held for `5! = 120`, but a script was
handed `120\n` as the number 120. Both are fixed in `8f905764`; see the review of the rerun below.

### Protocol attempts

Each cell is the task verdict, the task's wall time in seconds and what happened. A FAIL that
the table marks as expected is the outcome the fixes are meant to produce for a model that
cannot run the task.

| Attempt | Model and task | Round 1 (build A) | Round 2 (build B) | Round 3 (build B) | Result |
| --- | --- | --- | --- | --- | --- |
| F02 | llama3.1:8b factorial | FAIL, 99.9: steps and code as prose, no tool call | FAIL, 99.6: the same | Not run | Model limitation, unchanged |
| F03 | llama3.1:8b urine | FAIL, 59.7: a preface, then a JSON "call" naming an unrelated tool, never run | FAIL, 55.8: the same | Not run | Model limitation, unchanged |
| F04 | qwen2.5-coder:7b factorial | FAIL, 91.3: one checklist call, then imitated tool results; the corrective retry fired | FAIL, 75.9: one checklist call, then code and outputs written as prose | Not run | Model limitation; the retry fires when the reply is a whole imitation |
| F05 | qwen2.5-coder:7b urine | FAIL, 44.2: one checklist call, then imitated results; the retry fired | FAIL, 44.6: a checklist call written as text; the text-call retry fired, and the next reply repeated the call inside prose | Not run | Model limitation |
| F06 | gpt-oss:20b factorial | FAIL, 33.3: the checklist's arguments written as text, no tool ran | **PASS**, 134.0: 17 calls, 3 recovered errors | **PASS**, 72.5: 6 calls, no errors | Flaky model: two of three runs pass. The failed run made no tool call, so the build difference does not explain it |
| F07 | ministral-3:8b factorial | FAIL, 1,803 (time limit): the `Result: {}` defect above | FAIL, 1,125.9: correct file, then scripts that never passed a value to `record_result` despite the note saying so, until the repetition guard stopped the turn | FAIL, 912.8: first six `todo_write` calls; five wrote `-  [  ]` or `-   [ ]`, parsed as 0 items and were answered as successes until the loop guard fired (a Biorouter defect the review found, fixed in `65be50c5`). Then Python triple quotes in JavaScript five times despite the hint, then a direct call to `developer__text_editor`, which this chat does not offer; the turn ended on that clear error | Model limitation, with Biorouter defects in rounds 1 and 3 now fixed |
| F08 | granite4:tiny-h factorial | FAIL, 27.7: seven `todo_write` calls without `content`, ended by the loop guard | FAIL, 25.0: six, ended by the repetition guard | Not run | Model limitation, unchanged |
| F09 | phi4-mini:3.8b factorial | FAIL, 48.9: a prose answer with the right numbers, no tool call | FAIL, 45.0: the same | Not run | Model limitation, unchanged |
| F10 | lfm2:24b factorial | FAIL as expected, 9.9: the context sentence (32,768-token window, about 32,253 for instructions and tools) printed by `session send` and stored; the chat-title call then sent the model a second request | FAIL as expected, 9.2: the same | Not run | Fix confirmed twice; the title call is fixed in `8e186442` |
| F11 | deepseek-r1:8b factorial | FAIL as expected, 0.16: the turn refused before loading, naming the template that never shows tools; the chat-title call then loaded the model | FAIL as expected, 0.12: the same | Not run | Refusal confirmed twice; the title-call load is fixed in `8e186442` |
| F12 | command-r7b:7b factorial | FAIL as expected, 3.5: the context sentence (8,192-token window, about 31,473 for instructions and tools), printed and stored; the chat-title call then sent the model a second request | FAIL as expected, 3.0: the same | Not run | Fix confirmed twice; the title call is fixed in `8e186442` |
| F13 | glm-4.7-flash factorial | **PASS**, 354.1: 11 calls, 2 recovered errors | **PASS**, 358.1: 13 calls, 1 recovered error | **PASS**, 292.2: 8 calls, 2 recovered errors | Fix confirmed three times: 120 and 2432902008176640000 |
| F14 | gemma3:4b factorial (planned negative) | FAIL as expected, 0.82: refused, Ollama reports no tool support; the chat-title call then loaded the model | FAIL as expected, 0.75: the same | Not run | Expected refusal; the title-call load is fixed in `8e186442` |

F01, the Gemma survey, is a formal case and is reported below. Every passing pair also passed
the harness's pair checks (owned cleanup, exact unload, daemon gone, empty serving state).
Build B changes only the text shown for a Code Execution script that ran to its end, so rounds 1
and 2 exercise the same code for every attempt whose model ran no such script. The three whose
models did (F06, F07, F13) had a third run, so each has two on build B.

`biorouter session send` still exits 0 in every refused or failed turn above; the error is
printed as an `[error:…]` line and stored, and the exit status remains the CLI contract decision
noted in the follow-up.

### Formal cases

All 29 formal cases ran on build B, one exact test and one model per process: 35 invocations,
30 with a passing verdict, and every one of the 29 cases has at least one. The five failed
verdicts are all harness observation failures; in each, the product test itself reported
`1 passed`, and a retry with a new tag passed every check.

| Cases | Invocations | Result |
| --- | --- | --- |
| Ollama (`ollama_live`): the configure probe and the blocking completion on Qwen3.8 27B and Gemma 4 31B, and the reporter model's call written as text | 5 | All PASS, 2.6 to 8.8 seconds |
| Llama Server integration, Qwen3.8 27B: 11 cases | 14 | All 11 PASS. `an_over_window_prompt_is_a_context_error` failed its first verdict only on `no_guard`: the test process exited while the harness was proving its identity. `switching_models_restarts_sidecar` failed two verdicts only on `serving_provenance_captured`: the sidecar was stopped before it answered the harness's authenticated `/health` and `/props` reads |
| Llama Server integration, Gemma 4 31B: 11 cases | 13 | All 11 PASS. `an_over_window_prompt_is_a_context_error` failed two verdicts only on `no_guard`: the harness could not read the exiting test process's working directory |
| Survey, Qwen3.8 27B | 1 | PASS: 37,303 prompt tokens and a valid tool call from them |
| Survey, Gemma 4 31B (F01) | 2 | PASS twice: 39,096 prompt tokens and a valid tool call from them, 332 and 322 seconds |

F01 is therefore confirmed twice: the enlarged fixture from `dfd0db50` gives Gemma 39,096 prompt
tokens, above the 25,000-token threshold, and the tool call from that prompt is judged and passes.
Times are end-to-end under shared host load, not benchmarks.

Every pair and formal process was stopped by its owned identity. At the end no `llama-server`
process remained, the isolated Ollama on 11445 was stopped with its owner record removed, and
the user's Ollama on 11434 reported no loaded model.

### Harness observations

- **The refusal cases' cleanup failures were caused by Biorouter.** For F11 and F14 every pair
  records harness errors: serving provenance cannot prove a loaded model, because a correct
  refusal never loads one, and the pair's cleanup census fails on one process, an Ollama runner
  (`/Applications/Ollama.app/Contents/Resources/llama-server`). This section first blamed the
  harness's own `ollama stop`. The isolated Ollama's `serve.log` shows otherwise: about a second
  after each refusal, Ollama began loading the refused model's blob for a `POST /api/chat`
  (logged as 499 when the pair stopped the daemon), and the harness sends no `/api/chat`. It was
  Biorouter's chat-title call, which runs after every reply loop and carries no tools, so it
  passed the refusal. In F10 and F12 the same call sent the model a second request after the
  turn's own request had overflowed. Biorouter's daemon and its children
  were gone each time (`remaining_owned_children` is empty in every receipt). See the review of
  the rerun below for the fix and the rerun.
- **`session send` prints a streamed reply one chunk per line.** Each streamed chunk of a reply
  becomes its own `[assistant]` line (for example, ministral's sentences arrive a word or
  character at a time). The original campaign's stdout shows the same, and the PR changes only
  how notices are printed, so this is pre-existing behaviour of `session send`, outside these
  attempts.

## Review of the rerun (2026-10-12)

Independent reviewers then read the rerun's evidence and the code it ran. Two reviews confirmed
the claims above: each refusal and context fix, the F04/F05 retry, every protocol PASS against
its raw `session.json`, the formal verdicts, and green CI on `67402886`. Four findings were
Biorouter defects or wrong claims in this report. Each was reproduced, fixed with tests that fail
without the fix, and the attempts it touched were run again (below). Three commits on top of
`67402886` carry the fixes; the corrections to the sections above are made in place.

| Finding | What was wrong | Change |
| --- | --- | --- |
| A correct script was told its values were undefined | `acbefe5a` decided that a variable held a string from a per-run set of the tools that had ever returned one, and added that note to scripts that ran to their end. When one tool returned a JSON object on one call and text on the next, `info.name`, a real property of the object, was reported undefined. After `lines = lines.split("\n")`, `lines.filter` was reported as a member a string lacks. Both scripts recorded correct results and were told otherwise, which invites the model to redo correct work | `8f905764`: the hint is made only about a tool whose every result in the run was text, and only about a variable assigned once. A script that ran to its end gets the note only when the value it passed to `record_result` really held undefined values, and the note names them |
| A number a command printed reached the script as a number | A tool's text that parsed as a JSON number was handed over as a number, so `120\n` became 120 while 20! stayed text (`4010c2f9`). The hint "use `result5.trim()`" was then false and its fix threw, and two numeric outputs gave a bare `Result: {}`. The live F13 passes worked partly because 20! is above 2^53 | `8f905764`: only a JSON object or array is parsed; any other result is the tool's text, so one tool's results are one kind and every digit survives |
| The chat-title call reached a refused model (F10, F11, F12, F14) | `maybe_rename_session` runs after every reply loop and asks the model for a title with no tools, so it passed the R19 refusal. It loaded deepseek-r1:8b and gemma3:4b about a second after their turns were refused before loading, and sent lfm2:24b and command-r7b:7b a second request after their turns overflowed. That runner is what the F11 and F14 cleanup checks found; this report had blamed the harness | `8e186442`: after a turn that ends on a provider failure, the chat is titled from its first message without calling the model. A title a model wrote on an earlier turn is kept |
| `todo_write` reported success for a checklist it could not read (F07 round 3) | `-  [  ] Create…` and `-   [ ] Create…` matched nothing. Five writes were answered "Todo list set: 0 item(s)" as successes, each replacing the checklist with nothing, until the loop guard fired | `65be50c5`: spacing around and inside the brackets is ignored, and a write with text that names no item is refused with the expected format, leaving the existing checklist in place |

Verification of the three commits: eight new tests and one extended test cover the reviewer
probes. With the fixes undone, the eight that check a fix fail; the ninth checks that a complete
result gets no note and passes either way. With the fixes, all 4,969 `biorouter` library tests, the
`code_execution_integration` (42), `chatrecall_code_execution` (25),
`code_execution_module_invariants` (1) and `session_naming_tests` (3) binaries,
`cargo clippy -p biorouter --all-targets -- -D warnings` and `cargo fmt` pass.

### Live rerun of the affected attempts

The attempts the fixes touch were run again on a fresh debug build of `65be50c5` (build C), made
in a separate worktree on branch `pr393-work-7` with source identity `d9476b3b` over the
campaign's 22 compile-input paths, with the same harness and task prompts and a new owned
one-slot Ollama 0.40.3 on port 11445, one model at a time. F06 was added because its earlier
scripts were shown `out5: 120`. Evidence, with every executable hashed in `binary_receipt.json`,
is in `~/biorouter-runs/pr393-review-fix-reruns-2026-10-12/` under the new tags
`pr393fix-r{1,2,3}-20261012-protocol-NN`; the Ollama request counts are from that run's own
`serve.log`.

| Attempt | Model and task | Result at build C | What it shows |
| --- | --- | --- | --- |
| F10 | lfm2:24b factorial | FAIL as expected, 9.4 s: the context sentence printed and stored. Ollama received one `/api/chat`, the turn's own, answered 400; no title request followed | The title fix |
| F12 | command-r7b:7b factorial | FAIL as expected, 3.3 s: the same, one `/api/chat` | The title fix |
| F11 | deepseek-r1:8b factorial | FAIL as expected, 0.16 s: refused, naming the template. Ollama received no `/api/chat` and loaded no model, and the pair's `owned_cleanup_succeeded` and `post_cleanup_empty_state_proven` checks, which failed in every earlier round, pass. Only `serving_provenance_captured` still fails, because a correct refusal loads no model for it to prove | The title fix, and the corrected cause of the cleanup failures |
| F14 | gemma3:4b factorial (planned negative) | FAIL as expected, 0.78 s: refused, no tool support. The same as F11: no `/api/chat`, no load, both cleanup checks pass | The same |
| F13 | glm-4.7-flash factorial | **PASS**, 361.1 s: 14 calls, 1 recovered error. Its scripts were handed `"120\n"` and `"2432902008176640000\n"`, both text | The scalar change keeps F13 passing |
| F06 | gpt-oss:20b factorial | **PASS**, 65.8 s: 6 calls, 1 recovered error, and `out5: "120\n"` beside `out20: "2432902008176640000\n"`, where rounds 2 and 3 showed `out5: 120` | The scalar change |
| F07 | ministral-3:8b factorial | FAIL, 596.2 s: the checklist was written correctly the first time (4 items), then Python triple quotes in JavaScript five times despite the hint, then a direct call to `developer__text_editor`, which this chat does not offer | Model limitation, unchanged. The checklist defect did not recur because the model used standard spacing, so its fix is covered by tests |

At the end the isolated Ollama on 11445 was stopped by its owned identity, no `llama-server`
remained, and the user's Ollama on 11434 reported no loaded model.

## Limits to preserve in the final result

- The automatic memory reduction happens after a native reply starts. A load that fails before
  that point still needs an explicit smaller context or fewer parallel slots.
- Already resolved concurrent requests may retain their original window. A healthy observed
  window is not continuously monitored.
- Numeric model limits equal to the name-based default lack override provenance, including
  configured model-picker, API, and worker values. Explicit `OLLAMA_NUM_CTX` provider settings
  and `BIOROUTER_CONTEXT_LIMIT` process environment overrides bypass adaptive reduction;
  a saved `BIOROUTER_CONTEXT_LIMIT` value alone does not.
- Text-call recovery checks the advertised names and basic/nested argument types. It does not
  implement every JSON Schema constraint.
- The test named `switching_models_restarts_sidecar` ensures the same selected model twice.
  It does not request another model or witness a PID restart. Same-manager replacement across
  distinct models is unexercised by the finite roster (`20b7f913`), not a demonstrated defect.
- Local results alone do not establish other platforms or packaged Electron preview behavior.
  H1 CI supplies the explicitly scoped native results above; final-head status is recorded in
  [PR #393's current-head checks](https://github.com/BaranziniLab/biorouter/pull/393/checks).

## Related documentation

- [Investigation and handoff record](local-inference-handoff.md)
- [Provider documentation](../../providers/README.md)
