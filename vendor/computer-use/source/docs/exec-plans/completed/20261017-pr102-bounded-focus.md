# PR #102: bounded Linux focus lookup

## Objective

Merge latest main into #102, preserve #100 read-back behavior and all existing tests, review focus traversal bounds and real selection behavior, repair defects, validate on Linux and push follow-ups to the original PR. Do not merge.

## Constraints

Use the existing worktree and a temporary container on the already-running engine. Do not start UTM or operate personal desktop applications. Keep full tree/index and screenshot behavior, and distinguish incomplete focus search from complete absence. Keep requests synchronous with the existing outer bridge timeout; traversal budgets cannot interrupt a blocked AT-SPI call.

## Progress

- [x] Merge latest main; retain both action-read-back and focus tests and all release entries.
- [x] Add boundary regressions, inspect real GI text-selection APIs and repair any defects.
- [x] Run Linux Go/Python/Node tests, isolated real GTK focus/selection checks and existing action smoke.
- [x] Update docs/history, synchronize remotes, push and verify merge readiness.

## Validation

Check wide trees at the depth boundary, bounded node fetches, complete absence, focus found after a pruned branch, shared focus/selection lookup and error containment. Use native MCP with a disposable GTK fixture on a private Xvfb display and D-Bus session. Verify selection metadata and budget reporting while keeping existing action read-back tests passing. Record platform limits.

## Evidence and outcome

- Main `1b6fd8f` is integrated. The test conflict preserves #100 action read-back regressions alongside focus tests; release entries are retained.
- Four pre-fix regression failures established the depth-boundary fetch defect and unsupported selection API. All 24 Python tests now pass.
- Real GTK/native MCP focus smoke passes both budget limits, Unicode selection, text truncation and clearing selection back to a focus summary. Existing read-back smoke passes.
- Full Linux Go test binary and Node 24 36/36 passed in Debian 12 ARM64; Linux-targeted vet and both architecture builds passed.
- No desktop VM, personal app interaction, Wayland claim or whole-task speedup claim. Tracking budgets remain cooperative within the existing bridge timeout.
