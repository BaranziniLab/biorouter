# Built apps hardening verification

Implementation and native acceptance completed October 11, 2026. Merge readiness is
recorded by the [current PR checks](https://github.com/BaranziniLab/biorouter/pull/392/checks)
and the final verification record in [PR #392](https://github.com/BaranziniLab/biorouter/pull/392).
A passing older checkpoint does not replace checks on the current PR head.

Continue in `/Users/wgu/Desktop/BioRouter/.claude/worktrees/biorouter-built-apps-testing-cc226e`,
on `claude/biorouter-built-apps-testing-cc226e`. The upstream base is `30e1f0622`.
The original handoff was `a423670d5`; the completed payload implementation is
`cd56d90cb`, the Windows source-alias correction is `21b770408`, and the Crew fixture
readiness correction is `d74b2985d`. The main checkout and original QA profile were preserved.

Four isolated worktrees contributed reviewed fixes:

- [Security and lifecycle](built-apps-security-followup.md)
- [Export and recipient synchronization](built-apps-export-followup.md)
- [Build and browser serving](built-apps-build-followup.md)
- [Desktop navigation, authentication and file writes](built-apps-desktop-followup.md)

## Completed verification

Source review, generated-launcher fixtures, real browser tests and native application
acceptance are distinct evidence. Only observed results are marked passed below.

| Check                                 | Observed result                                                                                                                                                                                                                                                                                                                                                                  |
| ------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Interrupted reviews                   | All 21 findings have implementation dispositions in the four reports, with independent followup review.                                                                                                                                                                                                                                                                          |
| Combined required gates               | Nine completed `CARGO_BUILD_JOBS=2 just check-everything` runs passed, most recently at `d74b2985d`: formatter, strict clippy, UI lint, schema, version, brand, capability naming, cross drift and both registry checks.                                                                                                                                                         |
| Agent Drafter Rust namespace          | 369 passed, 2 ignored, 1,434 filtered after the knowledge-import correction. This filtered run does not claim coverage of the other crates.                                                                                                                                                                                                                                      |
| Full workspace Rust diagnostic rerun  | Core 4,726 passed / 2 ignored; ACP 17; bench 1; Crew 23; CLI 766; sandbox 21; server library 975 passed / 1 ignored; daemon binary 902 passed / 1 ignored. MCP had 1,795 passed / 6 ignored and one legacy fat-export fixture failure, subsequently corrected. Final platform-wide CI is tracked at the current PR head.                                                         |
| Full UI Vitest checkpoint             | [Run at `21b770408`](https://github.com/BaranziniLab/biorouter/actions/runs/38121608515): 754 files, 10,553 tests passed, 19 skipped; 400.97 seconds. A later Crew startup race was corrected without removing assertions; that file passed 53 tests and the previously failing case passed again. Final full-suite status belongs to the current PR head.                       |
| Focused UI checks                     | Export tests: 14 passed; captured-credential tests: 15 passed; source console census: 31 passed; repository UI Prettier check passed.                                                                                                                                                                                                                                            |
| SDK and compiler checks               | SDK DOM/WebSocket harness: 167 assertions passed in jsdom; pinned compiler packaging: 6 passed; export appcheck: 3 passed.                                                                                                                                                                                                                                                       |
| Integration tests                     | UI examples: 5 passed / 1 ignored; privacy wiring: 3 passed; real socket retirement: 5 passed, including actual TCP closure and token revocation after deletion.                                                                                                                                                                                                                 |
| Real daemon / Chrome policy           | Repeated after the payload rebuild: 12 assertions passed for authored scripts, window/form-scoped handlers, navigation cancellation, injected-handler refusal, binder replay refusal and cloned-template refusal.                                                                                                                                                                |
| Real CLI serve protocol               | 8 assertions passed: loopback authentication/exchange, no-store shell, plain HTTP LAN shell and exchange refusal, forged forwarding-header refusal, script-fetch refusal and modern HTTPS proxy navigation. The test listener bound only to loopback.                                                                                                                            |
| Native Launch and build chats         | Static and agentic Launch opened the daemon origin in the default browser. The BMI control changed the computed result. The final production rebuild read the fictional knowledge page through the real provider and returned `FINAL_LAUNCH_QA_OK`. Existing build chats opened. Deleting a backed-up chat in the isolated copy produced disabled “Build chat deleted” feedback. |
| Native managed preview                | A saved link to the old daemon port rebound to the new daemon origin after restart and reached Session ready. A real provider turn rendered the same-app donut frame visibly: A=1 (33.3%), B=2 (66.7%). Open in browser opened the new origin.                                                                                                                                   |
| Native error feedback and file writes | A missing app produced “Could not open the app” / “This app no longer exists.” Workflow and hints saves succeeded with expected bytes in the allowed project; refused targets produced failure feedback without changing the target. The disposable hints fixture was restored.                                                                                                  |
| All three native folder exports       | Launcher, Full and Full + bundled daemon completed through the native picker. Binary assets matched byte for byte, secret/workspace exclusion canaries were absent, source chat IDs were absent, and launchers were 0755.                                                                                                                                                        |
| Full/fat payload contents             | Both carried one selected custom skill and one authorized knowledge archive. The fat export paired the selected 393,256,360-byte daemon with native esbuild 0.27.3 (10,385,170 bytes); both were 0755.                                                                                                                                                                           |
| Fresh Full recipient                  | Executed `run.command` with only its own provider configuration initially present. First-run authenticated import registered the included base and installed the skill. Real `knowledge__kb_read_page` returned the fictional page phrase, then the provider returned `FULL_V2_RECIPIENT_QA_OK`.                                                                                 |
| Fresh fat recipient                   | Executed `run.command` with an OS-only PATH, no external daemon/compiler and no Node override. An obsolete SDK and edited source forced its bundled compiler to rebuild; the DOM showed the new marker. Included knowledge retrieval, real provider reply and the visible donut figure all passed; no browser errors were captured.                                              |
| Fresh Launcher recipient              | Initially showed its missing-dependency warning and returned a real provider reply. After independently installing the documented recipient prerequisites, the same thin export read the existing knowledge page successfully. Launcher exports intentionally carry no skill/knowledge payload.                                                                                  |
| Recipient replay                      | Full and fat reruns reused their own daemon. Knowledge registries and import receipts stayed byte-identical; no additional imports occurred.                                                                                                                                                                                                                                     |
| Generated launcher parity             | Actual Bash, Node and portable PowerShell templates passed authenticated import, collision/chained-ID mapping, partial failure/retry, consent, independent-edit preservation, skill backups and receipt tests. Windows short/long path replay additionally checks that PowerShell and Node share receipts.                                                                       |

The native recipient proof exercised actual knowledge registration and agent tool access;
staging an archive or reaching Session ready alone would not establish that result.
Test prompts and the knowledge verification page were fictional. The real provider was
Versa Azure with the configured GPT-5.5 model. No local model or mocked provider was used
for those recipient replies.

## Resolved diagnostic failures

Earlier Full/fat recipients reached Session ready but returned MissingKB because their
archives were only staged. `cd56d90cb` imports archives through the authenticated existing
daemon API, preserves privacy/ownership rules, maps allocated IDs and records per-archive
receipts before installing skills. Fresh functional reruns above passed.

Actual Windows PowerShell 5.1 first exposed unavailable `Get-FileHash` module discovery.
The streaming .NET hashing replacement passed all four Windows runtime-digest cases.
The next run showed PowerShell expanding an 8.3 source path while Node retained its short
spelling. `21b770408` uses native physical-path normalization and retains an alias replay
regression. The actual PowerShell 5.1 runtime-digest and full-payload steps both passed at
`21b770408`, including short-path and cross-launcher receipt reuse. Final whole-job status
still belongs to the current PR head.

A later full Vitest run exposed an unchanged Crew test clicking its synthetic access button
before channel observation settled. The fixture now awaits the observed channel before
one click; finished-state text, absence of Allow and the Open chat assertions remain.
The focused file passed; the final full-suite result is recorded on the PR.

The first full Rust diagnostic run also found HTTP seeded-bundle filtering, a Hermit
interpreter fallback in a proxy fixture, outer-profile test-root conflicts and a sandboxed
chmod fixture issue. Those were corrected before the clean rerun. None of these failed
runs is represented as a passing full suite.

## CI and artifact delivery gate

Require the current PR head's three platform Rust test jobs, full Vitest, no-ai-coauthor
and all other requested checks before marking the PR ready. Record completed run links,
source SHA and packaging assertions in the final PR verification record. Linux package
verification includes installing GUI/CLI packages in clean Debian/Rocky containers.
The installed CLI package compiler at `/usr/libexec/biorouter/esbuild` executes, reports
0.27.3 and compiles source into a nonempty bundle. The GUI compiler is separately
staged and checked as shipped payload; the workflow does not execute the compiler
extracted from the GUI package. A successful merge or source build alone is not a release claim.

## Controlled profile and retained evidence

Native tests used `/private/tmp/built-apps-final-qa/runs/gui`, copied from
`/Users/wgu/biorouter-runs/built-apps-qa`. Only the copy was modified. Recipient roots were
fresh temporary directories selected by `BIOROUTER_PATH_ROOT`; the real HOME stayed unchanged.
Private execution logs and sanitized result JSON are under `/private/tmp/built-apps-final-qa/`.
Credentials, launch access links and raw private logs are neither exported nor committed.

The debug-only `biorouter/privacy-test-auth` seam was used solely in controlled QA builds.
It must never be compiled into a release. The installed app and original QA profile were
not changed. Test daemons must be stopped only after verifying their exact PID and path.

## Deliberate design limits

Per-app origins/storage, app privacy classification and trusted identity stamps for legacy
build-chat records need separate designs. Browser isolation relies on modern navigation
metadata and opener support; legacy loopback clients without those headers remain supported.
Plain HTTP network addresses now refuse both app pages and the secret-bearing serve shell.
Use loopback or a correctly configured HTTPS deployment. Extension registry references are
recorded rather than universally bundled. Same-user parent-directory swaps and already
submitted background knowledge jobs need separate containment designs. Exports target the
exporting daemon's OS/architecture and require the recipient's own provider configuration.

See [browser access](../deployment/browser-access.md),
[serve decisions](../deployment/serve-decisions.md), and [privacy tiers](../security/privacy-tiers.md).
