# PR #103: Linux hidden-node details

## Goal

Integrate latest main into the contributor branch, review the opt-in hidden-node optimization, test real GTK/AT-SPI behavior in an isolated X11 session, fix issues and push follow-up commits to the same PR. Do not merge.

## Scope and constraints

- Default snapshots, node order, indexes, runtime paths and visible descendants must retain their behavior.
- Omit details only when AT-SPI explicitly reports a node as non-SHOWING. Unknown/failed state reads keep full details.
- Use a disposable container on the existing engine with private Xvfb/D-Bus, not a desktop VM or personal applications. Do not claim Wayland or whole-task performance coverage.
- Preserve the original contribution; update the existing history and feature documentation.

## Progress

- [x] Merge latest main; retain both test groups and release-note entries.
- [x] Review state handling, action compatibility and omitted property reads.
- [x] Add repeatable real GTK coverage and state-failure/negative-coordinate regressions; no functional fix required.
- [x] Run relevant Python/Go/native MCP checks and document evidence.
- [x] Synchronize remotes and prepare tested follow-up commits for the original #103 branch.

## Validation

Compare default and lightweight trees on real GTK controls with hidden/visible nodes, verify identity/index preservation and visible descendants, count actual property helper calls, and exercise native MCP actions with lightweight mode enabled. Include explicit unknown/failure fallbacks in unit tests. Run existing action read-back/focus smoke where compatible, Python and Linux Go suites, cross-builds, docs and whitespace checks. Container and fixture processes must be removed after testing.

## Evidence

- Main integrated through `eb501f4`, retaining original commit `917f2fe` and all upstream focus/read-back tests and release notes.
- Python 29/29; Linux Go 25/25; Linux ARM64/AMD64 builds and targeted vet passed. Docs/whitespace and shell/Python syntax checks passed.
- Real GTK 3 notebook on Debian 12 ARM64, private Xvfb/D-Bus and AT-SPI: 12 stable nodes, 3 hidden; four property-helper call totals 48 → 36. Helpers invoked real remote accessibles. These are helper-call counts, not D-Bus message counts or a latency benchmark.
- Native MCP compared disabled/opt-in output and indexes, clicked to reveal hidden controls, observed restored values/geometry, returned to the original page, then passed the existing action/no-read-back sequence. Separate default-action smoke and both disabled/enabled focus/Unicode-selection smokes passed.
- Temporary container, fixture processes and standalone build artifacts removed. No desktop VM, host personal-app actions, version bump, Wayland/other-toolkit validation or whole-task performance claim.
