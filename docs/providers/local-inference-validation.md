# Local inference final validation

> **Status:** Draft; in progress. Current local checks pass, but live verification is
> blocked by the mandatory disk floor. Completed checks and live outcomes retain their
> tested source scope, including model quality failures. Original investigation results are in the
> [handoff](local-inference-handoff.md).
> **Checkpoint scope:** Interim checkpoint before area-commit finalization and draft-PR delivery.

## Checkout and scope

- Worktree: `.claude/worktrees/ollama-llamacpp-json-bug-6cc4c2`.
- Branch: `claude/ollama-llamacpp-json-bug-6cc4c2`.
- Historical investigation base: `30e1f06221dfeb2322af99489722a7860783a2f3`.
- Retained delivery base, including the idle-timeout correction: `474f40087490ff3628d88b59be13a9995ff946e3`.
- Preserve the original research folder, `~/biorouter-runs/local-inference-2026-10/`.
- Resumed checks and live runs use separate evidence folders and freshly built binaries.

The final report will include whole-tree test counts, the CI-equivalent gates, live task
outcomes, binary and artifact hashes, commit-boundary compilation, and current PR checks.
No planned live task is counted as a pass.

Late code review found that a cached Ollama window could ignore a later explicit numeric
limit passed to the same provider instance, or let an explicit first call populate the
automatic cache. The correction and three request-body regressions are applied and formatted.
Local verification of that correction completed against source identity
`97554ff6b74dc8d9f891146efa6f690f7b55ba988ed9e5362032ff6dfb66b0d8`.
That revision is now historical: a later privacy correction has been applied. Its matching
workspace, gate and boundary checks pass, but remaining live verification is pending; earlier checks and live
runs below are not credited to it.

## Current privacy correction and remaining verification

Final review found that a Private Ollama session could dispatch a cloud alternate after
remembering an OpenAI-compatible fallback, bypassing the native request's refusal. The
applied guard checks the selected model before either transport and also checks discovered
remote aliases. Three new regressions cover remembered fallback, every request kind,
native alias discovery and the permitted Public-cloud control.

The formatted provider hash is
`00b64dd825210b93581423d5367ce921dc0884afc91f37ebb84c577dfaebc7e9`.
The new candidate is `e30868735cd15dae8a9ffdafa2773c8606919b6c`, with expanded source identity
`e9b21f6fffbc669451a3ef468756f0cf81d2da82fb7296da1a59b2a287e3cac9`.
The source snapshot is saved, and all six matching commit-boundary checks pass locked,
offline, whole-workspace/all-target Cargo checks with one job. All four required desktop
typecheck invocations and three landing JavaScript syntax checks exit zero. The temporary
checkout was removed with exit zero; the task branch and index, including raw index bytes,
stayed unchanged. The receipt is `candidate-boundaries-an92hyte/receipt.json` (`77800381`).
The current workspace library/binary run exits zero with 17 result summaries: 9,328 passes,
zero failures and 11 ignored tests. The core library has 4,951 passes and three ignored.
Verified provider subsets are Ollama 55, formats 322, Llama Server 34, sidecar 54 and toolshim 13.
The receipt is `late-override-validation-20261011T185146Z-355a2b19/workspace-final.receipt.json`
(`5e49872e`). Six context/privacy regressions, 55 Ollama tests, 46 affected core integration
tests, 25 local-route/server integration tests and seven privacy tests also pass. These
counts overlap the workspace and provider subsets and must not be summed.

All 21 current local gates pass, and their process exits zero. The canonical checks pass
with the unmodified two-job configuration; OpenAPI regeneration/drift and formatting pass.
The CLI, daemon and three live-test executables were freshly compiled, each with Cargo's
`fresh:false` record, and all five saved copies match their recorded source hashes.
The three default live-test guard invocations pass 11 non-model tests and ignore 15 model
tests; those ignored functions remain separately unrun. The validation receipt is
`late-override-validation-20261011T185146Z-355a2b19/validation-receipt.json` (`0a71a13a`),
and its binary receipt is `binary_receipt.json` (`31c007ac`).

Source, HEAD, staged content and flags remain unchanged. Raw index bytes changed through
stat metadata; that qualification is explicit, with no restoration or test rerun.

All five validated executables are now promoted into the live harness by a same-volume
atomic rename, without recopying from the mutable build cache. The active copied-binary
receipt is `binary_receipt.json` (`9f4d3202`), and the relocation receipt is
`late-binary-promotion-lfs998qz/relocation-receipt.json` (`f642c533`). The previous `04257524`
receipt and five executables are preserved in the live harness's
`bin-archive/042575-20261011T194922Z-cc22274444cf4300ac7ff7e1a7e3e89f/`;
the archive receipt is `old-five-bin-archive-20261011T194922Z-4abfc61f/archive-receipt.json`
(`55b5b2bb`).

Current harness preparation is rebound to control `e217e8e8`, observer `e2929121` and
aggregate `375a3ab7`. Its aggregate application receipt is `73967d78`; the prior privacy
aggregate `0d0869ba` remains archived. The seven focused timeout checks (`d28a4a21`) pass
separately from the unchanged 332 historical synthetic fixtures. Preparation adds no live cases.

The sealed current-source Gemma/Llama Server attempt (`97a119a0`) requested eight tasks,
invoked four and model-completed three. Two raw artifact checks and two strict independent
reviews pass: factorial and note counting. Loading fails model grounding/schema/final-claim
review. Querying is resource-interrupted without a final answer; its partial delete-tool
name proves neither execution nor mutation. Curation, labs, debugging and visualization
were not run. All 14 owned processes are absent, models/sidecars are empty and source/five
copies are unchanged; resource closure fails only the disk floor. The original failures
and nonzero campaign/observer exits remain preserved; no eight-task campaign pass is claimed.

Exact-owned node-cache pruning passes (`71671434`), preserving source, index, debug-cache
identity and all 15 retained executables. Disk remains below the 10 GiB live-test floor,
so this cleanup adds no live credit. The six Gemma recovery tasks and remaining 43-case
roster, durable evidence, history finalization and final-head remote CI remain pending.

| Tested source | Completed evidence | Remaining work |
| --- | --- | --- |
| Earlier `d06` | Local/frontend/integration checks and native-model campaigns below; their failures remain | No results are relabeled as a later revision |
| Context correction `97554ff6` | 18 successful gates, 9,325 workspace passes, six compiled boundaries, Gemma's three thinking-disabled cases, adaptive probe and managed Qwen's eight cases | Live quality and workflow failures remain explicit |
| Privacy correction `e9b21f6f` | Six compiled boundaries, 21 local gates, 9,328 workspace passes, affected regression/integration/privacy passes, five promoted executables, preparation rebind and two strict Gemma case passes | Disk-blocked recovery six and remaining 43-case roster; durable saving, finalization and final-head remote CI remain pending |

Remaining delivery work includes managed Gemma's campaign, cached reporter/protocol cases,
explicit model integration/survey invocations, the real workflow self-test, remaining chart inspections,
final documentation checks, exact-owned cleanup, verified database checkpointing, durable
evidence saving, commit organization, branch synchronization, the PR and final-head remote CI.
Six repeated Llama tasks with thinking disabled are conditionally omitted only if the
explicit formal suites establish the required off-mode coverage; that coverage is pending.
No planned or ignored model test is counted as an actual live pass.

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
recheck is recorded below; the full live campaign remains pending.

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
That local gate aggregate is green for the stated `d06` source. The subsequent provider
correction, remaining live campaign, final documentation/history cleanup, push and final-head
remote CI require their own verification. No planned model task is counted as a pass.

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

In progress. On the earlier `d06` frozen source, the first Qwen/Ollama default pair completed all
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
chart data and HTML. It runs one serving model at a time. Remaining GGUF lanes are scheduled
to use an owned one-slot Ollama with a 4096 server default after its control and live pilot
checks pass; the user's five-slot setting remains unchanged.
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
- Local results do not establish Windows/Linux behavior, packaged Electron preview behavior,
  or final remote CI. Those require the current-head workflow evidence.
