# Built apps hardening verification

Status: integration verification in progress, October 10, 2026.

The original handoff's implementation and interrupted review fixes are integrated in
`claude/biorouter-built-apps-testing-cc226e`, in
`/Users/wgu/Desktop/BioRouter/.claude/worktrees/biorouter-built-apps-testing-cc226e`.
The local and pushed implementation checkpoints both resolve to
`aff68dd6192f723a08525c17d7ef3b4da31d3ca6`. `origin/main` remains `30e1f0622`.
The main checkout and original QA profile were preserved.

Four isolated workstreams contributed the followups:

- [Security and lifecycle](built-apps-security-followup.md)
- [Export and recipient synchronization](built-apps-export-followup.md)
- [Build and browser serving](built-apps-build-followup.md)
- [Desktop navigation, authentication and file writes](built-apps-desktop-followup.md)

## Verification ledger

Only completed checks are marked passed. Source review is not runtime evidence.

| Check                                                               | Result                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Independent review of all 21 interrupted-review findings            | Implementation dispositions recorded in the four reports; runtime acceptance is tracked separately below                                                                                                                                                                                                                                                                                                                                                               |
| Combined `CARGO_BUILD_JOBS=2 just check-everything`                 | Six completed runs passed, including `f24ccb938`, `3c0a514e5` and `aff68dd61`                                                                                                                                                                                                                                                                                                                                                                                          |
| UI repository Prettier check, Node 24.10.0                          | Passed                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| SDK DOM/WebSocket control harness (jsdom)                           | 167 assertions passed; this does not execute figure scripts in Chrome                                                                                                                                                                                                                                                                                                                                                                                                  |
| Pinned esbuild packaging regressions                                | 6 passed                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Export appcheck regressions                                         | 3 passed                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| Workspace Rust libraries and binaries                               | Clean rerun completed: core 4,726 passed / 2 ignored; ACP 17; bench 1; Crew 23; CLI 766; sandbox 21; server library 975 passed / 1 ignored; daemon binary 902 passed / 1 ignored. MCP had 1,795 passed / 6 ignored and one legacy fat-export fixture failure, subsequently corrected; the affected Agent Drafter suite then passed 368 / 2 ignored / 1,434 filtered                                                                                                    |
| UI full Vitest suite                                                | [Corrected CI checkpoint `cd20bf3b2`](https://github.com/BaranziniLab/biorouter/actions/runs/38116708108): 754 files passed, 10,552 tests passed, 19 skipped (10,571 total). The `3c0a514e5` run is in progress. Focused current export tests: 14 passed; credential tests: 15 passed                                                                                                                                                                                  |
| UI examples, privacy wiring and real socket retirement integrations | `ui_example_apps`: 5 passed / 1 ignored; `privacy_guard_wiring`: 3 passed; `app_socket_lifecycle`: 5 passed, including actual TCP socket retirement and credential revocation after deletion                                                                                                                                                                                                                                                                           |
| Real daemon CSP/browser regression                                  | 12 Chrome assertions passed against a real binary: authored scripts/handlers, injected markup refusal, binder replay refusal and frame path. Final rebuild and repeat remain required after the recipient payload correction                                                                                                                                                                                                                                           |
| UI build and debug privacy-test binary                              | Passed at the tested checkpoint; repeat after the recipient payload correction                                                                                                                                                                                                                                                                                                                                                                                         |
| Electron Launch and build-chat navigation                           | Static and agentic Launch reached the daemon directly; static BMI interaction updated; real Versa GPT-5.5 replied and displayed the fictional donut chart. Existing build chats opened correctly; deleting a chat in the backed-up test copy produced disabled “Build chat deleted” feedback                                                                                                                                                                           |
| In-chat managed preview                                             | The native chat link reached Session ready on the daemon origin. A real provider turn completed the figure tools; the same-app figure frame appeared in the accessibility tree. After scrolling, the native preview visibly rendered the donut colors and legend: A=1 (33.3%), B=2 (66.7%). Open in browser successfully focused the existing Chrome app tab                                                                                                           |
| Native Launcher, Full and Full + bundled daemon exports             | All three native folder exports completed; binary assets matched byte for byte, exclusions held, and launcher modes were 0755. The fat export contained the selected 393 MB daemon and 10 MB native esbuild 0.27.3                                                                                                                                                                                                                                                     |
| Fresh Launcher recipient                                            | Passed with its own provider configuration and a fresh root; the real agent replied                                                                                                                                                                                                                                                                                                                                                                                    |
| Fresh Full and fat recipients                                       | Both reached Session ready and imported the selected skill. Included knowledge payloads were only staged, and dependent calls returned MissingKB. API-based import correction and complete functional reruns remain required                                                                                                                                                                                                                                           |
| Fat recipient native compiler                                       | With an OS-only PATH and no compiler override, an obsolete SDK and edited source forced a real rebuild; the resulting DOM confirmed the new compiled marker. Final repeat remains required after the payload correction                                                                                                                                                                                                                                                |
| Platform CI, including Windows PowerShell                           | [PR #392](https://github.com/BaranziniLab/biorouter/pull/392) remains a draft. At `3c0a514e5`, all five native compiler staging jobs and Linux cross checks passed; full Rust CI is in progress. Windows failed before Rust at the portable PowerShell check because Get-FileHash required an unavailable module. `aff68dd61` uses .NET stream hashing; four actual portable PowerShell checks pass locally. Windows PowerShell 5.1 and current-head CI remain pending |

The source-based console census passes all 31 checks. The captured launch credentials
are now checked structurally, and the focused credential suites pass all 15 tests.
The assembled website generator also passes its check. These corrections do not
substitute for the full UI suite or the native acceptance checks below.

The earlier full Vitest run at `658ed8c4` had 753 files passed and one failed;
10,551 tests passed, one failed and 19 were skipped. Its sole failure was the
formatting-sensitive captured-credential assertion. The structural repair retained
the owning-window and captured-secret checks; the later `cd20bf3b2` full run passed.

The first full Rust run passed 4,726 core tests (2 ignored), 765 CLI tests, 1,790 MCP
tests (6 ignored), 970 server library tests (1 ignored) and 897 daemon binary tests
(1 ignored). The failures were: one lock-mode fixture affected by sandboxed chmod,
one proxy fixture that accidentally fell back from a Hermit script interpreter to a
real daemon, and the HTTP shipped-bundle filter in each server target plus four
test-root guards in each. These counts document the diagnostic run and are not a
passing-suite claim.

The clean MCP rerun's remaining legacy fat-export fixture failure was corrected in
`68f88b9b7` by exercising the public packaged-compiler contract. The later complete
Agent Drafter namespace rerun passed as recorded above; this is not a claim that
another full workspace rerun has completed after every subsequent source change.

The bundled export followup now stages the selected daemon's own native esbuild,
verifies the staged binary reports pinned version 0.27.3 within five seconds, and
publishes both binaries together. A missing or rejected helper produces an explicit
thin-export note with no partial daemon payload. Backend and desktop extraction
restore executable modes for both binaries. The fresh recipient test must force a
rebuild without an external compiler to verify this path; that check has now passed
for the tested fat export. Knowledge import is a separate unresolved acceptance
failure and prevents marking Full/fat recipient functionality complete.

## Remaining acceptance work

- Finish the API-based import of included knowledge bases, rebuild the UI and debug
  daemon, and repeat affected browser and recipient checks. Full and fat recipients
  must use their included knowledge base successfully and receive real provider replies.
- The native managed-preview figure and Open in browser success path passed on
  the tested binary. Repeat affected checks after the final rebuild, and complete
  browser failure feedback and allowed/denied hints/workflow file-write checks if
  their live evidence has not yet been captured.
- Capture completed full Vitest and Rust CI at the final pushed commit, including the
  native Windows PowerShell 5.1 digest regression and final combined required gates.
- Push the final integrated checkpoints and reconcile this ledger with the final
  PR/CI results. No final shipping or release claim is made by this checkpoint.

## Controlled live profile

The live test copy is `/private/tmp/built-apps-final-qa/runs/gui`, copied from the original
`/Users/wgu/biorouter-runs/built-apps-qa` profile. Only the copy is used for writes and
disposable lifecycle tests. Test input is fictional. The provider is Versa Azure, using
the configured GPT-5.5 model; no local model is used. Credentials are neither printed
nor included in exports or committed evidence.

Recipient tests keep the real `HOME` unchanged and select fresh temporary roots through
`BIOROUTER_PATH_ROOT`. The debug-only `biorouter/privacy-test-auth` seam may be armed for
controlled QA; it must never be included in a release artifact.

Private execution logs are under `/private/tmp/built-apps-final-qa/`. Only sanitized
counts and observed behavior are recorded here; private identifiers, credentials,
access links and raw logs are omitted.

## Deliberate remaining design limits

Per-app origins, privacy classification of apps and trusted identity stamping for legacy
build-chat records require separate designs. The interim browser isolation uses navigation
metadata and opener policy. Plain HTTP on a network address now refuses both app pages and
the secret-bearing serve shell; use loopback or a correctly configured HTTPS deployment.
See [browser access](../deployment/browser-access.md),
[serve decisions](../deployment/serve-decisions.md), and
[privacy tiers](../security/privacy-tiers.md).
