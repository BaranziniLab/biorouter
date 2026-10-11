# Built apps hardening verification

Status: integration verification in progress, October 10, 2026.

The original handoff's implementation and interrupted review fixes are integrated in
`claude/biorouter-built-apps-testing-cc226e`, in
`/Users/wgu/Desktop/BioRouter/.claude/worktrees/biorouter-built-apps-testing-cc226e`.
The implementation checkpoint is `46c6fd3de`. `origin/main` remains `30e1f0622`.
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
| Independent review of all original open implementation findings | Closed; dispositions are in the four reports |
| Combined `CARGO_BUILD_JOBS=2 just check-everything` | First combined pass passed; final checkpoint rerun in progress |
| UI repository Prettier check, Node 24.10.0 | Passed |
| SDK DOM/WebSocket control harness (jsdom) | 167 assertions passed; this does not execute figure scripts in Chrome |
| Pinned esbuild packaging regressions | 6 passed |
| Export appcheck regressions | 3 passed |
| Full Rust library suites | Pending |
| UI full Vitest suite | Pending; the earlier interrupted partial run is not a pass |
| UI examples, privacy wiring and real socket retirement integrations | Pending |
| Real daemon CSP/browser regression | Pending on rebuilt final binary |
| Electron Launch, build chat, in-chat preview and browser opening | Pending |
| Native Launcher, Full and Full + bundled daemon exports | Pending |
| Fresh recipient execution and real agent responses | Pending |
| Platform CI, including Windows PowerShell | Pending |

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
