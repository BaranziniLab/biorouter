# Built apps hardening verification

Status: integration verification in progress, October 10, 2026.

The original handoff's implementation and interrupted review fixes are integrated in
`claude/biorouter-built-apps-testing-cc226e`, in
`/Users/wgu/Desktop/BioRouter/.claude/worktrees/biorouter-built-apps-testing-cc226e`.
The implementation checkpoint is `194485c3a`. `origin/main` remains `30e1f0622`.
The main checkout and original QA profile were preserved.

Four isolated workstreams contributed the followups:

- [Security and lifecycle](built-apps-security-followup.md)
- [Export and recipient synchronization](built-apps-export-followup.md)
- [Build and browser serving](built-apps-build-followup.md)
- [Desktop navigation, authentication and file writes](built-apps-desktop-followup.md)

## Verification ledger

Only completed checks are marked passed. Source review is not runtime evidence.

| Check | Result |
|---|---|
| Independent review of all 21 interrupted-review findings | Implementation dispositions recorded in the four reports; runtime acceptance is tracked separately below |
| Combined `CARGO_BUILD_JOBS=2 just check-everything` | Passed three times; latest run includes the browser console-window census correction |
| UI repository Prettier check, Node 24.10.0 | Passed |
| SDK DOM/WebSocket control harness (jsdom) | 167 assertions passed; this does not execute figure scripts in Chrome |
| Pinned esbuild packaging regressions | 6 passed |
| Export appcheck regressions | 3 passed |
| Full Rust library suites | First combined run completed with 4 failed targets. The HTTP bundle filter and Hermit Node fixture are corrected. The lock fixture passes outside the tool sandbox; a clean per-process-profile full rerun is in progress |
| UI full Vitest suite | [Corrected CI checkpoint `cd20bf3b2`](https://github.com/BaranziniLab/biorouter/actions/runs/38116708108): 754 files passed, 10,552 tests passed, 19 skipped (10,571 total). The final compiler extraction change needs current-head CI; its 14 focused export tests passed |
| UI examples, privacy wiring and real socket retirement integrations | Pending |
| Real daemon CSP/browser regression | Pending on rebuilt final binary |
| Electron Launch, build chat, in-chat preview and browser opening | Pending |
| Native Launcher, Full and Full + bundled daemon exports | Pending |
| Fresh recipient execution and real agent responses | Pending |
| Platform CI, including Windows PowerShell | [PR #392](https://github.com/BaranziniLab/biorouter/pull/392) is open as a draft. Three frontend failures (assembled documentation, visible browser census and a formatting-sensitive credential test) have local corrections; current-head CI is required |

The source-based console census passes all 31 checks. The captured launch credentials
are now checked structurally, and the focused credential suites pass all 15 tests.
The assembled website generator also passes its check. These corrections do not
substitute for the full UI suite or the native acceptance checks below.

The first full Rust run passed 4,726 core tests (2 ignored), 765 CLI tests, 1,790 MCP
tests (6 ignored), 970 server library tests (1 ignored) and 897 daemon binary tests
(1 ignored). The failures were: one lock-mode fixture affected by sandboxed chmod,
one proxy fixture that accidentally fell back from a Hermit script interpreter to a
real daemon, and the HTTP shipped-bundle filter in each server target plus four
test-root guards in each. These counts document the diagnostic run and are not a
passing-suite claim.

The bundled export followup now stages the selected daemon's own native esbuild,
verifies the staged binary reports pinned version 0.27.3 within five seconds, and
publishes both binaries together. A missing or rejected helper produces an explicit
thin-export note with no partial daemon payload. Backend and desktop extraction
restore executable modes for both binaries. The fresh recipient test must force a
rebuild without an external compiler to verify this path.

## Controlled live profile

The live test copy is `/private/tmp/built-apps-final-qa/runs/gui`, copied from the original
`/Users/wgu/biorouter-runs/built-apps-qa` profile. Only the copy is used for writes and
disposable lifecycle tests. Test input is fictional. The provider is Versa Azure, using
the configured GPT-5.5 model; no local model is used. Credentials are neither printed
nor included in exports or committed evidence.

Recipient tests keep the real `HOME` unchanged and select fresh temporary roots through
`BIOROUTER_PATH_ROOT`. The debug-only `biorouter/privacy-test-auth` seam may be armed for
controlled QA; it must never be included in a release artifact.

Private execution logs are under `/private/tmp/built-apps-final-qa/`. This document will
record final counts and observed behavior so those temporary logs are not the only evidence.

## Deliberate remaining design limits

Per-app origins, privacy classification of apps and trusted identity stamping for legacy
build-chat records require separate designs. The interim browser isolation uses navigation
metadata and opener policy. Plain HTTP on a network address now refuses both app pages and
the secret-bearing serve shell; use loopback or a correctly configured HTTPS deployment.
See [browser access](../deployment/browser-access.md),
[serve decisions](../deployment/serve-decisions.md), and
[privacy tiers](../security/privacy-tiers.md).
