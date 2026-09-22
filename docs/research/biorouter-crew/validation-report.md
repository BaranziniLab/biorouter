# Crew validation report

Validation was run in `/Users/wgu/.codex/worktrees/biorouter-crew/BioRouter` on
branch `codex/biorouter-crew`, with Rust revision `26f2a496` and current HEAD
`8f2ea8df5778e7e8c4e8b08e62bcd31d6bcdbf57`. No AWS source or
binary export was used.

## Checks and focused regressions

- `source bin/activate-hermit && CARGO_BUILD_JOBS=2 just check-everything` —
  passed all ten checks, including Rust clippy, the socket inheritance gate,
  UI lint/typecheck/theme/contrast/token checks, OpenAPI generation/schema,
  version/brand/naming, vendored-source, cross-drift, registry, and privacy
  registry checks.
- `cargo test -p biorouter-crew --test broker_contract` — 22 passed.
- `cargo test -p biorouter worker_scope_validation --lib` — 1 passed.
- `cargo test -p biorouter failed_policy_save_preserves_connection_and_scope_authority --lib` — 1 passed.
- `cargo test -p biorouter-server --lib observer_gets_snapshot_then_live_events` — 1 passed.
- `cargo test -p biorouter-server --lib reconnect_after_a_missed_finish_receives_authoritative_idle_state` — 1 passed.
- `cargo test -p biorouter workspace_list_pages_instead_of_truncating --lib` — 1 passed.
- `cargo test -p biorouter background_compaction_of_a_private_transcript_on_a_public_provider_is_refused --lib` — 1 passed.
- `npm run test:run -- src/components/crew/CrewView.regression.test.tsx` — 4
  passed (action-error retention across manual refresh and background poll
  failure/recovery, stable retry identity, and human restart/remount gates).
- `npm run lint:check` — passed typecheck, ESLint, theme, contrast, and token
  checks after the UI regression fixture was corrected to use UUID-shaped IDs.

## Fresh daemon

`CARGO_BUILD_JOBS=2 cargo build -p biorouter-server --bin biorouterd`
completed successfully. The checked worktree daemon is:

```text
target/debug/biorouterd
sha256: 8f0113378714acb7308ce36fd506e145ac26009ed454aae31575e4d8530abed9
size: 374168088 bytes
```

## Bounded local-model self-test

The loopback Ollama inventory was queried with:

```text
curl --fail --max-time 5 -sS http://127.0.0.1:11434/api/tags
```

It reported only `qwen3:1.7b`, digest
`8f68893c685c3ddff2aa3fffce2aa60a30bb2da65ca488b61fff134a4d1730e7`.

The bounded run used an isolated profile, the private loopback endpoint, and no
credentials:

```text
BIOROUTER_PATH_ROOT=/tmp/biorouter-selftest-luna-positive
BIOROUTER_PROVIDER=ollama
OLLAMA_HOST=http://127.0.0.1:11434
OLLAMA_TIMEOUT=30
timeout 180 target/debug/biorouter run --workflow biorouter-self-test.yaml \
  --model qwen3:1.7b --params test_phases=basic \
  --params test_depth=quick --params cleanup_after=true
```

The process exited 1 with `Error: the turn did not complete: tool_loop` after
the model repeatedly issued `workspace_list`. This is recorded as a failed
self-test; the model/tool loop did not provide a passing workflow result.

## Limits

The disposable AWS fixture remains owned by the fixture lifecycle record and
must be cleaned up by its deadline. Source export approval remains absent, so
no source or product binary was transferred to AWS.
