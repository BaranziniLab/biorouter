# Crew validation report

The initial validation below ran in `/Users/wgu/.codex/worktrees/biorouter-crew/BioRouter`
on branch `codex/biorouter-crew`, with Rust revision `26f2a496` and then-HEAD
`8f2ea8df5778e7e8c4e8b08e62bcd31d6bcdbf57`. Later sections identify their own
artifacts and results, including the final local check at `367fe588`.
No AWS source or product binary export was used.

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

## Follow-up validation (2026-09-22)

The scoped request regressions passed with four selected tests: the omitted
connection ID reached a fake `remote.list` target, an explicit wrong ID was
refused, null/number/object IDs were refused, and an omitted ID without a
grant was refused. Three control-path regressions passed for long `TMPDIR`,
private ownership/mode, and symlink collision denial. The two existing Crew
provider-sampling regressions passed after moving path-root relocation into an
isolated test process. The three hosted-CI source guards each passed with one
selected test:

```text
privacy::system_auth::tests::no_caller_raises_a_prompt_without_a_bound_on_it — 1 passed
test_sandbox::tests::only_the_resolver_and_the_sandbox_read_the_path_root_variable — 1 passed
test_sandbox::tests::only_the_sandbox_relocates_the_path_root_and_only_in_a_process_of_its_own — 1 passed
```

Strict Clippy passed with `cargo clippy -p biorouter --lib --tests --no-deps
-- -D warnings`, and `cargo fmt --all -- --check` passed. A fresh
`biorouter-server` build completed with `CARGO_BUILD_JOBS=2` and
`CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target`; the source and
worktree daemon hashes match:

```text
sha256: 1ffa69d980d4664b452b2c492100af96fd4055ba3d0e875f076b331fc83b8d86
version: biorouter-server 1.91.1
```

The current `just check-everything` run did not complete. It stopped during
Clippy on the newly added `crates/biorouter-crew/tests/adversarial_contract.rs`
with six `-D warnings` findings (`let_unit_value` and
`redundant_closure_call` at lines 217, 403, and 585). The earlier failed
bounded local-model self-test above remains a failure; this follow-up does not
replace or reinterpret it.

## AWS export limitation and cleanup

Source export approval remained absent, so no source or product binary was
transferred to AWS. The disposable fixture was subsequently cleaned up at
13:40 UTC as verified below; it is not an active acceptance environment.

## Final lifecycle verification

At 13:40 UTC, the exact recorded cleanup command completed with status
`verified`. Independent follow-up checks against the state-recorded resources
confirmed instance `i-0b2896cc361853bf7` is `terminated`, EBS volume
`vol-084c96be40d83a693` returns `InvalidVolume.NotFound`, security group
`sg-0b6976a48c2117188` returns `InvalidGroup.NotFound`, and bootstrap key pair
`bootstrap` returns `InvalidKeyPair.NotFound`. The fixture directory contains
only `fixture-state.json` and `user-data.b64`; no private-key or public-key
files remain. Local Docker and GUI fixtures were not touched. The then-current
post-merge `just check-everything` and four-test Crew UI regression run passed,
superseding the earlier incomplete-check note for that revision. Later product
changes have their own validation below.

## Current validation refresh (2026-09-22)

After the adversarial test cleanup, the repository gate completed successfully:

```text
source bin/activate-hermit && CARGO_BUILD_JOBS=2 just check-everything
all style, Rust, UI, schema, version, registry, privacy, and drift checks passed
registry tests: 61 passed; privacy-registry tests: 21 passed
```

The fresh CLI build also completed in the reused target:

```text
source bin/activate-hermit && CARGO_BUILD_JOBS=2 \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  cargo build -p biorouter-cli
artifact: /private/tmp/biorouter-crew-target/debug/biorouter
sha256: 2661b979eb5dba004a1be8d1c7aa0030722c64bdea925cab9ded8ca4d0c7fcb4
architecture: Mach-O arm64
version: biorouter-cli 1.91.1
```

A read-only documentation check covered 27 Markdown files under this report's
Crew documentation tree: zero broken local links, zero unclosed fenced blocks,
and zero trailing-whitespace lines. At that point the isolated 8b self-test was
queued; its later failed and zero-exit-without-assertions outcomes are recorded
below, neither as a passing workflow.

## Linux portability refresh (2026-09-22)

The broker-only cross build was run through the pinned `rust:1.92-bullseye`
x86_64 toolchain using the documented Bash heredoc (the helper depends on
Bash's `BASH_SOURCE` when resolving the worktree root):

```text
bash <<'BASH'
set -euo pipefail
source scripts/cross-env.sh
cross_linux 'cargo build --locked --release -p biorouter-crew -j 2' /usr/src/myapp/target/crew-portable
BASH
```

The resulting artifact is an x86_64 ELF at
`target/crew-portable/x86_64-unknown-linux-gnu/release/biorouter-crew`, with
SHA-256 `2a72d02df3d57b9785305d3bdf978d24f4e2cb248652ba8a459983ca8335594d`.
In a Debian 11 x86_64 container under macOS emulation, UID 65532 ran
`--version`, `--help`, and an invalid command. The version was
`biorouter-crew 1.91.1`; the invalid command returned nonzero. ELF imports
reached GLIBC 2.30, below the pinned 2.31 floor. DT_NEEDED contained only
`libgcc_s.so.1`, `libpthread.so.0`, `libdl.so.2`, `libc.so.6`, and
`ld-linux-x86-64.so.2`.

The per-binary glibc guard passed with max 2.30, and the runtime-dependency
and nfpm payload guard passed with no undeclared non-glibc dependency. Both
guards rejected a malformed `biorouter-crew` third artifact when the other two
fixture binaries were valid, confirming a failure in that binary cannot be
masked. Bash syntax checks, `check-no-cross-drift`, `cargo fmt --all -- --check`,
and `git diff --check` passed.

The same pinned artifact also passed a bounded rootless service smoke in a
disposable Debian 11 x86_64 container under macOS emulation: UID 65532 used a
private HOME/state directory and synthetic root-owned machine identity;
`start`, `status`, and a bridge-mediated Unix-socket `hello` returned matching
workspace/UID/socket metadata and private mode. The lifecycle `stop` command
was attempted with the exact recorded PID under both default Docker seccomp
and seccomp-unconfined containers, but both returned
`unsupported: safe stop requires Linux pidfd_open`. This is recorded as an
emulated-kernel limitation; native Linux is required to qualify the exact-pid
stop path. No remote execution or network qualification is claimed.

## Fresh CLI local-model self-test (2026-09-22)

The fresh CLI artifact
`/private/tmp/biorouter-crew-target/debug/biorouter` (SHA-256
`2661b979eb5dba004a1be8d1c7aa0030722c64bdea925cab9ded8ca4d0c7fcb4`) was run
with an isolated synthetic profile at
`/private/tmp/biorouter-selftest-luna-8b-20260922`, private loopback Ollama,
model `qwen3:8b` (digest
`500a1f067a9f782620b40bee6f7b0c89e17ae61f686b92c24933e4ca4b2b8b41`), and a
180-second bound:

```text
BIOROUTER_PATH_ROOT=/private/tmp/biorouter-selftest-luna-8b-20260922
BIOROUTER_PROVIDER=ollama
OLLAMA_HOST=http://127.0.0.1:11434
OLLAMA_TIMEOUT=30
timeout 180 /private/tmp/biorouter-crew-target/debug/biorouter run \
  --workflow biorouter-self-test.yaml --model qwen3:8b \
  --params test_phases=basic --params test_depth=quick \
  --params cleanup_after=true
```

The run started session `20260922_1` but exited 70 with provider failure:
`Request failed: Stream decode error: error decoding response body`, followed
by `Error: the turn did not complete: provider_failure`. The full log is
`/private/tmp/biorouter-selftest-luna-8b-20260922.log`. This remains a failed
self-test and provides no Crew MCP or end-to-end workflow evidence.

The single higher-timeout retry used a new isolated profile at
`/private/tmp/biorouter-selftest-luna-8b-20260922-retry`, the same fresh CLI,
`OLLAMA_TIMEOUT=120`, and a 240-second total bound. It exited 0 after one
qwen3:8b response (60,314 ms; 15,599 input, 782 output, 23,923 total tokens;
733 chunks), but the CLI emitted no assistant report or workflow assertion.
The persisted session evidence contains one user message and one empty
assistant message, no tool messages, no checkpoints, no message blobs, and one
token event. The banner classified the session as Public; Ollama itself was a
local private endpoint/provider. This is a model-response/zero-exit result,
not a passing self-test or Crew MCP proof. The earlier 30-second timeout
`provider_failure` remains recorded above.

## Census wiring refresh (2026-09-22)

The focused census rerun used the shared target with incremental compilation
disabled and two build jobs:

```text
source bin/activate-hermit && CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  cargo test -p biorouter --test privacy_guard_wiring -- --test-threads=1
```

Result: 3 tests passed, 0 failed. The console-window census then ran with the
same environment:

```text
source bin/activate-hermit && CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  cargo test -p biorouter-mcp --test no_console_window_census -- --test-threads=1
```

Result: 19 tests passed, 0 failed. Formatting and whitespace checks also
passed:

```text
source bin/activate-hermit && cargo fmt --all -- --check
git diff --check
```

The privacy registry audit added `excluded_crew_sessions` with its five live
callers (`GET /schedule/{id}/sessions`; `GET /sessions/sidebar`,
`GET /sessions/insights`, and `GET /sessions/activity`; and
`GET /sessions/changes`). It also reconciled
the existing rows against current production calls: `session_reach` now records
agent 7/7, Crew 1/1, session 8/13, and session-events 2/2 (calls/refs), while
`http_caller` records session 5/0. The console census exempts only the two
Crew modules declared under `#[cfg(unix)]` in `biorouter-crew/src/lib.rs`; the
cross-platform core Crew spawn sites remain covered by the live assertion. The
new Crew guard caller is `POST /crew/connections/{id}/sessions/{session_id}/grant`
(`grant_session`), which requires human proof before attaching conversation
authority.

These are local source-census results. They do not qualify Windows desktop
acceptance; the hosted Linux rerun remains required for that lane.

## Final local repository check at `367fe588` (2026-09-22)

The validation lane completed the full repository command with the explicit
shared target, incremental compilation disabled and two build jobs:

```text
source bin/activate-hermit && CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 just check-everything
```

Result: **passed all ten checks**. The production socket-inheritance gate
checked 17 targets with zero warnings; registry tests passed 61/61 and
privacy-registry tests passed 21/21. This final local run supersedes the
earlier incomplete local-gate state and is distinct from hosted CI and
graphical agent acceptance.

The refreshed daemon used for ongoing GUI acceptance is SHA-256
`3133646fa1fc6b6369677f0c6b769b7ead5ce1bd145bf803e38bea29d3f71cc3`.
The corrected private Ollama API tool/result evidence and four public-sink
cases in [the provider report](local-provider-boundary-report.md) used the
earlier daemon `1ffa69d9…`; they do not establish a new-daemon graphical pass.
Historical CLI model failures and the empty zero-exit response remain recorded
above. No full end-to-end model, native named-Save, Windows desktop or AWS
product acceptance is inferred from the repository gate.

## ToolActivity projection unit refresh (2026-09-22)

The focused unit suite was run against the shared target with incremental
compilation disabled and two build jobs:

```text
source bin/activate-hermit && cargo fmt --all && \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  cargo test -p biorouter-server --lib routes::crew::tests -- --test-threads=1 && \
  git diff --check
```

Result: **5 passed, 0 failed, 712 filtered out**. The tests cover matching
Crew requests and responses, typed job receipts versus `Err(ErrorData)`
failures, same-request and same-response deduplication, and unmatched response
ids. They also prove that synthetic private success/error payloads never enter
Crew activity text, and that unknown methods and non-Crew requests are ignored.

The shared branch still contains the concurrent Crew implementation's 367+
line uncommitted source delta; Luna's change here is test-only coverage in
`routes/crew.rs`, with no product-logic change or durability claim.

## Scoped local-context regressions and artifact refresh at `373acdf5` (2026-09-22)

The focused regressions ran against the shared target with incremental
compilation disabled and two build jobs:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --lib authoritative_crew_context_omits_local_moim_context -- --nocapture
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --lib ordinary_context_retains_local_moim_and_prompt_context -- --nocapture
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --lib crew_prompt_omits_local_hints_while_ordinary_prompt_keeps_them -- --nocapture
```

Result: **1 passed, 0 failed, 4191 filtered out** for each command. The Crew
MOIM test proves a real scoped session omits the local path, workspace-map
canary, and `AGENTS.md` canary while retaining Crew remote guidance. The
ordinary-session control retains the local path and workspace-map canary. The
reply preparation test uses the actual created `Session.id` for the Crew
scope, proves the local hint is absent there, and proves it remains present for
an ordinary session.

Formatting and whitespace checks passed:

```text
source bin/activate-hermit && cargo fmt --all -- --check
git diff --check
```

The committed daemon and CLI were rebuilt from `373acdf5` with the same shared
target settings. The current daemon artifact is
`/private/tmp/biorouter-crew-target/debug/biorouterd`, version `1.91.1`,
SHA-256
`6c19b87f661f78a65011b33c00951206aecc14a19cbe75dab6d88a53ae1ca5ef`; the
worktree `target/debug/biorouterd` was replaced atomically and has the same
hash. The current CLI artifact is
`/private/tmp/biorouter-crew-target/debug/biorouter`, version `1.91.1`,
SHA-256
`3fe16d8f49650aef87565ba79cb45bd33a8f8e486111b7329c796d50bfa37f87`.

The required final local command passed all ten checks at `373acdf5`:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 just check-everything
```

This current artifact supersedes the earlier local daemon hash
`3133646fa1fc6b6369677f0c6b769b7ead5ce1bd145bf803e38bea29d3f71cc3`.
The historical `1ffa69d9…` daemon was used for earlier provider-boundary
evidence; neither historical artifact is the current `373acdf5` build.

## Run cancellation ledger regressions (pending final source commit, 2026-09-22)

The focused server route tests ran in the shared target with incremental
compilation disabled and two build jobs:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter-server routes::crew::tests:: --lib -- --nocapture
```

Result: **11 passed, 0 failed, 0 ignored, 0 measured, 712 filtered out** in
0.17 seconds. The 11 selected tests include the five existing ToolActivity
unit tests and six cancellation regressions. The cancellation fixtures use a
real temporary `RunLedger`, persist and read its JSON status where durable,
and subscribe to the session bus with a bounded event wait. They cover both
reservation-before-success and success-before-cancellation races; queued and
progress updates after reservation; failed remote revocation followed by the
same-endpoint retry; interrupted and `outcome_not_durable` retry states; real
ledger persistence failure reporting; and the already-completed `complete`
final event. These are helper/state/event tests, not live HTTP or network
cancellation tests.

`cargo fmt --all` and `git diff --check` passed after the test additions. The
route test process is idle; no daemon rebuild or installation was performed
for this test-only validation.

## Final committed snapshot gate and artifacts at `aac5f4ac` (2026-09-22)

After the cancellation route regressions and the two Crew core tests were
committed as `aac5f4ac`, capacity was 206 GiB free and no Rust process was
active. The required repository gate passed with the shared target, disabled
incremental compilation, and two build jobs:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 just check-everything
```

All ten checks passed, including strict clippy, the non-inheritable socket
check (17 production targets, zero ordinary warnings), UI typecheck/lint,
OpenAPI generation/schema comparison, version/brand/Copilot/cross-drift checks,
registry 61/61, privacy registry 21/21, and consistency.

The final native artifacts were built from `aac5f4ac` with:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo build -p biorouter-server --bin biorouterd \
    -p biorouter-cli --bin biorouter
```

Build passed. `biorouterd --version` reported `biorouter-server 1.91.1`, and
`biorouter --version` reported `1.91.1`; `--help` exited successfully for both.
The shared-target hashes are:

```text
1ce50cb31f63dca70c7bb25c571facd1672fe9271801ddbf5e82f5785483e407  /private/tmp/biorouter-crew-target/debug/biorouterd
af43d934f73ae14c3708da5eb5b9febc6ce3516f5fcbc93a9e8a4e7fb24cc44c  /private/tmp/biorouter-crew-target/debug/biorouter
```

Both artifacts were copied to temporary sibling files and atomically renamed
over the worktree destinations `target/debug/biorouterd` and
`target/debug/biorouter`; staged, source, and installed hashes matched. GUI
clients were not restarted, so their running processes remain separate from
this newly installed artifact.

## Final Crew documentation and harness hygiene (2026-09-22)

A read-only hygiene pass covered the 14 pending tracked or untracked paths under
`docs/research/biorouter-crew`. It found:

- Python AST parsing: 15 files, 0 syntax errors.
- JSON parsing: 4 files, 0 parse errors.
- Markdown fences: 29 files, 0 unbalanced fence pairs.
- Relative Markdown links: 73 checked, 0 broken. External, anchor, `codex://`, and `app://` links were excluded from the local-file check.
- `git diff --check`: passed.
- Pending text files contained 0 private-key-block, AWS-access-key, or SSH-private-key markers. No credentials or session-state files were added by this pass.

The selected pending evidence screenshots inspected with image preview were
`/private/tmp/biorouter-crew-ui-attachments/synthetic-crew.png`,
`downloaded-blocks-320x180.png`, `synthetic-blocks-320x180.png`,
`/private/tmp/carol-fresh-ui.png`, `/private/tmp/bob-current-ui.png`,
`/private/tmp/bob-owned-task.png`, `/private/tmp/bob-personal-crew-grant.png`,
`/private/tmp/carol-processing-correction.png`, and
`/private/tmp/carol-agent-final.png`. Eight were readable and showed only
synthetic UI/task data, identifiers, hashes, or local fixture paths; no private
key or credential material was visible. `synthetic-crew.png` rendered as a
uniform black image and is not readable evidence. No evidence screenshots are
committed under the Crew research tree. No fixtures, UI profiles, or model
services were mutated during this pass.

## Ignored Crew evidence-image review (read-only, 2026-09-22)

The repository evidence directory was inventoried with `rg --files --no-ignore`: 28 PNGs were present under `docs/research/biorouter-crew/evidence/`. Every file decoded successfully as an RGB PNG at 1440x1000. Contact-sheet review covered all 28 images; full-size review included the fresh-control member/result views, Carol connection/PAM cancellation views, and Bob personal/Crew views. No repository evidence image was blank or uniformly black. The known one-pixel `synthetic-crew.png` fixture is outside this directory and remains historical fixture input, not repository screenshot evidence.

The following 26 unique evidence files are approved for reference in reports. The evidence directory is ignored, so this is a review list; no image was staged or committed by this pass:

```text
alice-cross-channel.png
bob-current-ui.png
bob-personal-crew-grant.png
bob-personal-greeting.png
bob-personal-ready.png
bob-slash-existing2.png
carol-agent-final.png
carol-agent-session.png
carol-blocks-preview.png
carol-blocks-uploaded.png
carol-crew-home.png
carol-fresh-pam-after-password.png
carol-fresh-pam-prompt.png
carol-fresh-ui.png
carol-opaque-uploaded.png
carol-pam-cancelled.png
carol-png-preview.png
carol-png-uploaded.png
carol-processing-correction.png
carol-reconnected.png
final-9411-reconnected.png
final-9412-reconnected.png
final2-9411.png
final2-9412.png
fresh-control-members.png
fresh-control-result.png
```

Two files remain historical but are excluded from the unique reference list because they are byte-identical duplicates of `carol-agent-final.png` (SHA-256 `5bc5937ed37121c26fa14d26f47f96675806eeca0dbbcf2745f85005d1a64104`): `carol-personal-crew-after.png` and `final-9413-reconnected.png`. They were not deleted. The screenshots show synthetic UI/task content, local fixture paths, public enrollment-key metadata, and result hashes; no private-key body or credential/password value is visible.

A separate read-only SSH fixture inspection found the Carol helper at mode `0700`, owned by the fixture Carol UID, size 1748 bytes, SHA-256 `901605725a653a05544cd83bb7af79f711321475fb6d812a91cb991cc7f544d6`; `crew-task.csv` was mode `0644`, same owner, 31 bytes, SHA-256 `b8d8853f57f79b6bc9e4c3736b9f7c840e0975ea0606d31cc934349d5606335a`; and no `python3`, helper, or job-ID process was running at inspection time. Existing `summary.csv` and `fixture-check-summary.csv` were preserved.

## Live HTTP Crew cancellation regression (2026-09-22)

The bounded `cancellation-http` scenario in
`smoke/local_provider_boundary.py` was run once against the freshly installed
daemon, using a new temporary profile root, a loopback OpenAI-compatible mock
provider, and the disposable SSH broker fixture. It did not use any of the
three UI profiles, Ollama, or a model call. The exact command was:

```text
python3 -m py_compile docs/research/biorouter-crew/smoke/local_provider_boundary.py && \
python3 docs/research/biorouter-crew/smoke/local_provider_boundary.py --scenario cancellation-http
```

The daemon SHA-256 was
`1ce50cb31f63dca70c7bb25c571facd1672fe9271801ddbf5e82f5785483e407`; the
fixture broker SHA-256 was
`7311f126d0e77f6b7e6f4b75e0ac32b099ff38b4e7757ca04e92737e9ffbc759`. The
scenario printed `api_harness: pass` and exercised two mock-provider requests:

- A held streaming run (`930769d6-b497-4a28-b520-4fc1fab2b9f4`) first refused
  cancellation through `wrong-luna-owner` with HTTP 400 and the exact
  ownership refusal `This task is not owned by this device and connection.`.
  Cancellation through the owning connection returned HTTP 200 with
  `cancelled: true`, `status: cancelled`, and
  `remote_revocation_confirmed: true`.
- The session observer received exactly one terminal `Finish` SSE frame with
  `reason: cancelled`. Releasing the held provider stream afterward left the
  persisted run status `cancelled`, proving a late provider completion did not
  overwrite the cancellation.
- A second run (`4cd39ba6-6ad1-4e71-9142-709be8f7069b`) completed before its
  cancel request. The cancel response was HTTP 200 with
  `already_finished: true`, `status: completed`, and `cancelled: false`.

This is live route evidence for confirmed cancellation, owner mismatch,
completion-before-cancel, persisted terminal status, and the matched observer
event. The synthetic fixture does not force an unconfirmed remote revocation;
that failure-and-retry behavior remains covered by the six focused Rust route
tests recorded above. The mock server logged one expected client connection
reset after the daemon stopped the held upstream request; the scenario itself
completed with exit status 0 and all status-bearing assertions passed.

## Repository gate after remote FIOCLEX fix (2026-09-22)

The required gate was run on the current working tree after the reviewed
11-line `crates/biorouter-crew/src/remote.rs` FIOCLEX fix. Capacity before the
run was 206 GiB free, zero swap, and load averages 4.75/6.34/7.07. The exact
command was:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  just check-everything
```

It passed all ten checks: Rust formatting; strict and baseline clippy; the
non-inheritable production socket gate (17 targets, zero ordinary warnings);
UI typecheck, lint, themes, contrast (404 assertions), and tokens; OpenAPI
schema; version consistency; brand consistency; Biorouter Copilot naming;
vendored computer-use source; cross-compile drift; registry (61/61); and
privacy/consistency (21/21). The command exited 0. `git diff --check` also
passed afterward.

Source fingerprint for this gate: committed `HEAD`
`aac5f4acdc11da8c0a5b7b95d4054a2850a46ed8`; current uncommitted
`crates/biorouter-crew/src/remote.rs` SHA-256
`937cce9f67dd71d11775f2ba73d5b029aa01c331be0247012719cca5288f7f2c`.
No daemon or CLI rebuild was performed because this change is confined to the
broker crate.

## Linux arm64 FIOCLEX regression artifact (2026-09-22)

The earlier artifact at `/private/tmp/crew-linux-target/debug/biorouter-crew`
was excluded from acceptance evidence because it was a macOS arm64 Mach-O
binary (SHA-256
`d32208e1ada515b3c7d9f348e6679b1f0bcee2334108ce796c338c31b7b0ddc3`), which
also explains its disposable Linux canary exit 126. A fresh Linux build used
the cached `rust:latest` image, digest
`sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`,
which is Debian 13.6 arm64. The cached toolchain versions were `rustc 1.98.1
(48a229cea 2026-09-01)` and `cargo 1.98.1 (797e8a9bc 2026-08-05)`. The exact
build command was:

```text
docker run --rm --platform linux/arm64 \
  -v /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter:/workspace \
  -v /private/tmp/crew-linux-cargo-target-luna:/cargo-target \
  -w /workspace \
  -e CARGO_TARGET_DIR=/cargo-target -e CARGO_INCREMENTAL=0 \
  -e RUSTUP_TOOLCHAIN=1.98.1-aarch64-unknown-linux-gnu \
  -e PATH=/usr/local/cargo/bin:/usr/local/rustup/toolchains/1.98.1-aarch64-unknown-linux-gnu/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
  rust:latest cargo build -p biorouter-crew --target aarch64-unknown-linux-gnu
```

The resulting artifact at
`/private/tmp/crew-linux-cargo-target-luna/aarch64-unknown-linux-gnu/debug/biorouter-crew`
is an executable ELF 64-bit LSB PIE ARM aarch64 binary with SHA-256
`a38b34be38201fe6a1aacd21451d4c8b9311cc3478fd9d919f36810a037d83f5`. Its
highest imported GLIBC symbol is `GLIBC_2.39`.

The exact disposable harness command was:

```text
python3 docs/research/biorouter-crew/smoke/local_file_script_regression.py \
  --binary /private/tmp/crew-linux-cargo-target-luna/aarch64-unknown-linux-gnu/debug/biorouter-crew \
  --container biorouter-crew-regression-test-luna \
  --docker /usr/local/bin/docker --expect-file-script allow
```

It passed in the separate `biorouter-crew-regression-test-luna` container.
The exact helper argv was `["python3", "summarize_csv.py", "crew-task.csv",
"summary.csv"]`; stdout was `row_count=3 total=60\n`, and the output bytes
were exactly `row_count,total\n3,60\n` (21 bytes, SHA-256
`29f857b262032224f20e849e76bed6907e8e02846835a632c8380b2bafb91ab3`). The
network, subprocess, unknown ioctl (`0x12345678`), FIONCLEX (`0x5450`),
outside-file read, `/etc/passwd`, and `../.ssh` path negatives all failed with
the expected permission refusal. The test container was stopped and removed;
the live canary and three-user regression containers were left untouched.

The test-only harness source fingerprint is
`ae63572f7657eaed0f15035ae0cac25669796032b1f9dce385f30e93b9ef3e9a`, and the
synthetic fixture source fingerprint is
`901605725a653a05544cd83bb7af79f711321475fb6d812a91cb991cc7f544d6`.
`git diff --check` passed after recording this evidence.

## Rootless live fixture broker upgrade (2026-09-22)

After the UI lane paused model calls, a fresh process check found no active
remote helper jobs in `biorouter-crew-ssh-luna`; only Alice's broker and the
known idle bridge sessions were present. The verified ELF was uploaded through
each ordinary SSH account using its distinct fixture key. For each user, the
operation used a temporary path under that user's `.local/bin`, verified the
SHA-256 over SSH, set mode `700`, and atomically renamed the temporary file:

```text
scp -q -o BatchMode=yes -o IdentitiesOnly=yes -o IdentityAgent=none \
  -o StrictHostKeyChecking=yes \
  -o UserKnownHostsFile=/private/tmp/biorouter-crew-ssh-fixture/known_hosts \
  -i /private/tmp/biorouter-crew-ssh-fixture/keys/$user -P 56928 \
  /private/tmp/crew-linux-cargo-target-luna/aarch64-unknown-linux-gnu/debug/biorouter-crew \
  $user@127.0.0.1:/home/$user/.local/bin/.biorouter-crew.a38b34be3820.tmp

ssh ... "$user@127.0.0.1" \
  'got=$(sha256sum /home/$user/.local/bin/.biorouter-crew.a38b34be3820.tmp | cut -d" " -f1); \
   test "$got" = a38b34be38201fe6a1aacd21451d4c8b9311cc3478fd9d919f36810a037d83f5; \
   chmod 700 /home/$user/.local/bin/.biorouter-crew.a38b34be3820.tmp; \
   mv -f /home/$user/.local/bin/.biorouter-crew.a38b34be3820.tmp \
         /home/$user/.local/bin/biorouter-crew'
```

Alice, Bob, and Carol each reported the new artifact SHA-256
`a38b34be38201fe6a1aacd21451d4c8b9311cc3478fd9d919f36810a037d83f5`, mode
`700`, and ELF 64-bit ARM aarch64 type. Their ordinary SSH identity checks
returned UIDs 1101, 1102, and 1103 respectively.

The existing Alice broker was stopped only through Alice's stateful stop
command, which returned workspace ID `0fb6e0c6-32c0-4d0a-957e-8a76240d54c2`.
It was restarted as Alice with the new binary and the existing state directory,
without a bootstrap key or state reset. The new broker PID was `4891`. Runtime
metadata preserved host UID `1101`, node ID
`3469076c14385846f21a17d3e823e621c09cbaa33b73a32560cb31323d0bcd3b`, socket
`/tmp/crew-1101-d9f258fd97f241a3bf55a6b56b20e0e5/broker.sock`, the same
workspace ID, workspace key fingerprint
`a4489c000171b2f739dc571d42fa2911259500d674c181b97a59748456eb8f8a`, and the
same workspace public key. Existing profile, enrollment, journal, and history
state were preserved. The Bob bridge exited during the broker restart; no Bob
or Carol broker was restarted, and the Alice bridge remained idle.

## Pinned x86_64 Bullseye portability refresh (2026-09-22)

This refresh was built after the current FIOCLEX source change. The source
fingerprint for `crates/biorouter-crew/src/remote.rs` was
`937cce9f67dd71d11775f2ba73d5b029aa01c331be0247012719cca5288f7f2c`. Using the
pinned `rust:1.92-bullseye` image (`rust@sha256:c6d501c039204c21e9fa374f234bd41bdc8b36cfd455a407ef145d9bef19f2b7`)
with two cargo jobs produced:

```text
/private/tmp/crew-linux-portable-luna/x86_64-unknown-linux-gnu/release/biorouter-crew
SHA-256 eefaece19ce99b3ba54f7358c64a6d0ed4efb6788b37a8b080fefe3b7a9cb3e4
BuildID a7bbd1aa01020ae37fbd942978ad2caa1547df35
ELF64 x86-64 PIE, interpreter /lib64/ld-linux-x86-64.so.2
highest imported GLIBC symbol GLIBC_2.30
DT_NEEDED libgcc_s.so.1 libpthread.so.1 libdl.so.2 libc.so.6 ld-linux-x86-64.so.2
```

The artifact was exercised in the disposable `rust:1.92-bullseye` container
`biorouter-crew-portable-smoke-luna` as ordinary UID `65532` (not root). The
synthetic machine ID was valid hexadecimal and the state parent was private to
that user. Exact CLI outcomes were:

```text
--version: biorouter-crew 1.91.1
--help: Usage: biorouter-crew serve|start|status|stop --state-dir PATH [--bootstrap-key HEX]
        biorouter-crew bridge --stdio --socket PATH --owner-uid UID --workspace-id UUID
        biorouter-crew --version
        Broker and bridge operations require Linux.
start: {"started_pid":181,"state":"starting","status_command":"status"}
status: {"host_uid":65532,"node_id":"3469076c14385846f21a17d3e823e621c09cbaa33b73a32560cb31323d0bcd3b","pid":181,"protocol":1,"socket":"/tmp/crew-65532-eaf34679c115467986f7b98075fd0284/broker.sock","workspace_id":"0d18c3da-90d7-4e47-9dba-58c653fdd0cc","workspace_key_fingerprint":"090794a6c9f8ec14fbf65846e47c4653d684b737c84b5915f0443cb17ab22032","workspace_public_key":"5ece1f49812a442e68929029dcac983db2e2a5a03937bfb78de430569fbe0074"}
hello: response id hello-1, protocol 1, host_uid 65532, mode private, workspace_id 0d18c3da-90d7-4e47-9dba-58c653fdd0cc, capabilities human_chat/signed_devices/resumable_blobs/scoped_runs, unsupported arbitrary_shell/remote_filesystem/network_filesystem/cross_workspace_release
```

The process was observed as UID `65532` inside the disposable container. This
is a Debian Bullseye container/emulated x86_64 smoke, not evidence of a native
Debian 11 host or a remote SSH deployment. The exact disposable container was
removed after capture; the live three-user fixture and its profiles were not
touched.

## Provider-boundary broker artifact validation (2026-09-22)

The local provider-boundary harness no longer has a Mach-O-prone broker default.
`--broker-binary` is now explicit and is validated before the sink, temporary
profile, daemon, Docker fixture, or remote state is created. The validator
accepts only little-endian ELF64 x86_64 or aarch64 artifacts, which covers the
native Debian aarch64 fixture as well as the pinned x86_64 floor.

The lightweight command checks were run after the edit:

```text
python3 -m py_compile docs/research/biorouter-crew/smoke/local_provider_boundary.py
python3 docs/research/biorouter-crew/smoke/local_provider_boundary.py --scenario cancellation-http
python3 docs/research/biorouter-crew/smoke/local_provider_boundary.py \
  --scenario cancellation-http \
  --broker-binary /private/tmp/crew-linux-target/debug/biorouter-crew
```

Results: syntax check exited 0; omission of `--broker-binary` exited 1 with an
actionable request for a Linux ELF64 broker path; the known Mach-O artifact
exited 1 with `broker binary is not an ELF file` and an actionable Linux ELF64
replacement; and no fixture setup occurred in either rejection case. An
offline helper check using temporary 20-byte ELF headers accepted both
`x86_64` and `aarch64` machine ids. No build, daemon, model, or live fixture
was started for these checks, and the existing live fixtures were preserved.

## Refreshed Linux broker artifact and contracts (2026-09-22)

The ARM64 Linux artifact was rebuilt from current uncommitted broker source
SHA-256 `af3a1efdd43ad168a162ea2379bf76cfedc88a607ac864148678677494b3ef6e`
and committed FIOCLEX source SHA-256
`937cce9f67dd71d11775f2ba73d5b029aa01c331be0247012719cca5288f7f2c`, at
commit `3145dfc504ec51d92d299dfdc493669e61457bbd`. The cached Debian 13.6
arm64 `rust:latest` image and toolchain were unchanged (`rustc 1.98.1`,
`cargo 1.98.1`). The build used the existing isolated target directory,
`CARGO_INCREMENTAL=0`, `CARGO_BUILD_JOBS=2`, and target
`aarch64-unknown-linux-gnu`.

The refreshed artifact at
`/private/tmp/crew-linux-cargo-target-luna/aarch64-unknown-linux-gnu/debug/biorouter-crew`
is ELF 64-bit LSB PIE ARM aarch64, executable mode `755`, size 26,884,752
bytes, BuildID `3caf9f6d0fc1196bced7358e556e1788c3704a20`, and SHA-256
`4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`.
Its highest imported GLIBC symbol is `GLIBC_2.39`.

The Linux broker contract command was run in a disposable ARM64 container with
the synthetic machine-id prerequisite and two build/test jobs. The suite
passed 4/4 `adversarial_contract`, 23/23 `broker_contract`, and 4/4
`cursor_contract` tests: 31 total tests, 30 passed, 0 failed, and 1 ignored.
The ignored `journal_fault_contract` test requires its disposable Linux
LD_PRELOAD interposer fixture. The initial run without the synthetic machine
ID had 26 passes and one environment-only `node_identity_unavailable` failure;
the rerun passed the complete contract suite.

The refreshed file-script command ran against a newly named disposable
container, `biorouter-crew-regression-refresh-luna`:

```text
python3 docs/research/biorouter-crew/smoke/local_file_script_regression.py \
  --binary /private/tmp/crew-linux-cargo-target-luna/aarch64-unknown-linux-gnu/debug/biorouter-crew \
  --container biorouter-crew-regression-refresh-luna \
  --docker /usr/local/bin/docker --expect-file-script allow
```

It passed the exact helper argv with stdout `row_count=3 total=60\n`; output
bytes were `row_count,total\n3,60\n` (21 bytes, SHA-256
`29f857b262032224f20e849e76bed6907e8e02846835a632c8380b2bafb91ab3`). The
network, subprocess, unknown ioctl `0x12345678`, FIONCLEX `0x5450`,
outside-file, `/etc/passwd`, and `../.ssh` negatives all failed with the
expected permission refusals. The disposable container was stopped and
removed. No live fixture installation or restart was performed during this
refresh.

## Repository gate refresh (2026-09-22)

The required generated-schema command was run with Hermit after the initial
sandbox cache-access refusal:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 just generate-openapi
```

Server OpenAPI generation completed and wrote `ui/desktop/openapi.json`, but
the frontend generation step failed with exit 1 because the generated schema
contains a missing reference: `#/components/schemas/Resume`. No OpenAPI JSON
was hand-edited. The generated schema diff is 540 added lines; the frontend
client generation did not complete.

The full gate was then run with:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 just check-everything
```

Rust formatting completed. Clippy then exited 101, so later checks in the
ten-check recipe did not run. The blocking diagnostics were both in the
current uncommitted `crates/biorouter/src/crew/authentication.rs`: a
`clippy::collapsible_if` at line 306 and
`clippy::items_after_test_module` at line 394. No product fixes were applied
in response. `git diff --check` passed after the gate attempt. Existing
uncommitted source changes from the parallel Crew lanes were preserved.

## Headless Linux CLI and daemon artifacts (2026-09-22)

The CLI and daemon were built from the read-only working tree using the
immutable cached `rust:latest` arm64 image
(`rust@sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`),
with `CARGO_BUILD_JOBS=2`, `CARGO_INCREMENTAL=0`, target
`aarch64-unknown-linux-gnu`, and a separate Cargo target and home. `Cargo.lock`
was SHA-256 `ec966981755fa832c56f7bf22502dfd73bcc7629923f0965d112530a5de4ddc5`
both before and after the locked build. Critical source fingerprints at build
time were broker.rs `af3a1efdd43ad168a162ea2379bf76cfedc88a607ac864148678677494b3ef6e`,
remote.rs `937cce9f67dd71d11775f2ba73d5b029aa01c331be0247012719cca5288f7f2c`,
CLI main `3130ea2cfe220b7f205a49651eccb2172fdefacee19d696853398d4c657c6fa0`,
and server main `45c2777616e2ce3cff73f96c4d3c54ac8bede014344525e7377eeb92f749e70f`.

The exact build selected only the two requested binaries:

```text
docker run --rm --platform linux/arm64 \
  -v /Users/wgu/.codex/worktrees/biorouter-crew/BioRouter:/workspace:ro \
  -v /private/tmp/crew-linux-cli-daemon-target-luna:/cargo-target \
  -v /private/tmp/crew-linux-cli-cargo-home-luna:/cargo-home \
  -w /workspace -e CARGO_TARGET_DIR=/cargo-target -e CARGO_HOME=/cargo-home \
  -e CARGO_INCREMENTAL=0 -e CARGO_BUILD_JOBS=2 \
  -e RUSTUP_TOOLCHAIN=1.98.1-aarch64-unknown-linux-gnu \
  -e PATH=/usr/local/cargo/bin:/usr/local/rustup/toolchains/1.98.1-aarch64-unknown-linux-gnu/bin:$PATH \
  rust@sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082 \
  /usr/local/cargo/bin/cargo build --locked \
  -p biorouter-cli --bin biorouter -p biorouter-server --bin biorouterd \
  --target aarch64-unknown-linux-gnu
```

The resulting debug artifacts are native ARM64 ELF PIE binaries from
`rustc 1.98.1` / `cargo 1.98.1`:

```text
/private/tmp/crew-linux-cli-daemon-target-luna/aarch64-unknown-linux-gnu/debug/biorouter
  SHA-256 9e8ba2a3b525021ff385639dbdc2e3b5052d85f26f9282cbeb1fddb51a6b69c9
  size 1,373,891,624 bytes; BuildID dab7e6a5b472f95b991871b2a14530347ec0047e
  highest imported GLIBC symbol GLIBC_2.39

/private/tmp/crew-linux-cli-daemon-target-luna/aarch64-unknown-linux-gnu/debug/biorouterd
  SHA-256 3955a5b1b1abeeefeb5aba29f3aa73e150dc4fa5499c40a8435e56a5a8d22319
  size 1,490,646,696 bytes; BuildID 3fddc00c9aeede0e9fb3759af5b14f09005cbad1
  highest imported GLIBC symbol GLIBC_2.39
```

Inside the immutable Linux image, `biorouter --version` printed `1.91.1` and
`biorouterd --version` printed `biorouter-server 1.91.1`. Read-only headless
help checks with an isolated `HOME` and `BIOROUTER_DISABLE_KEYRING=1` completed
for `doctor --help`, `serve --help`, and `crew --help`, confirming the Linux
CLI's no-keystore command surface without starting a daemon, model, or live
profile. No live fixture or installed binary was changed by this build.

The same new CLI was also exercised under synthetic Linux UIDs 1101, 1102,
and 1103 in a disposable container, with an isolated `HOME` and
`BIOROUTER_DISABLE_KEYRING=1`. Each principal returned its expected UID,
`biorouter --version` printed `1.91.1`, and `biorouter crew --help` exited 0.
These were CLI-only probes with no daemon, provider, model, keystore, or live
fixture access.

## Latest repository gate refresh (2026-09-22)

After the Resume schema registration and unique Crew operation-id fixes, the
exact regeneration command was rerun:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 just generate-openapi
```

The server schema and frontend client generation both completed successfully
(`openapi-ts` 0.90.10). The earlier missing `Resume` reference was not
reproduced. The OpenAPI check still exits 1 until the generated
`ui/desktop/openapi.json` and `ui/desktop/src/api/` changes are included in
the pending source change; no JSON was hand-edited.

The latest full gate used the same bounded Hermit environment and exits 101 at
clippy. The sole Rust diagnostic is
`crates/biorouter-cli/src/daemon_client.rs:403`,
`clippy::redundant_async_block`, for
`tokio::spawn(async move { connection.await })`. No product fix was applied
in this lane.

Independent checks after that gate were: version consistency pass, brand
consistency pass, Biorouter Copilot naming pass, cross-drift pass, registry
61/61 tests pass with all outputs current, and privacy-registry 21/21 tests
pass with all copies agreeing. UI lint exits 1 with eight errors and one
warning: existing `NodeJS`/`RequestInfo` global typing diagnostics,
`no-control-regex` in `ui/desktop/src/main.ts:4871`, and a missing
`onConnected` effect dependency in `CrewAuthentication.tsx:130`. These are
reported as failures rather than promoted to green. No live fixture or profile
was changed. `git diff --check` remains required after generated files are
staged by the owning lane.

## Synthetic Linux 50-UID broker soak refresh (2026-09-22)

A fresh disposable ARM64 Docker fixture exercised the standalone broker with
50 synthetic real UIDs for the requested 1,800-second workload. The verified
ELF artifact was `/private/tmp/crew-linux-cargo-target-luna/aarch64-unknown-linux-gnu/debug/biorouter-crew` with SHA-256
`4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`; the harness
`docs/research/biorouter-crew/smoke/local_real_uid_soak.py` had SHA-256
`02d9d4f9d8fa1a61bd3e08f9752772ab45884debc0fd9d6dbfe7e6a21690970a`. The cached image digest was
`sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`. The fixture used a four-CPU cap and no network.

The run sent and acknowledged `1500` messages, read back
`1500` unique IDs, and reported zero disconnects or
harness errors. Opaque cursor pagination was exercised during the workload and
after same-state restart; replay matched all `1500`
expected body hashes (`replayed=1500`,
`missing=[]`, `mismatched=[]`). Broker maxima were RSS
`1260260` KiB, `106` open
file descriptors, and `2233383` journal bytes.

All users, keys, messages, and identifiers were synthetic. The run was
standalone broker evidence and did not exercise the GUI, daemon, providers,
installed profiles, or the existing three-user SSH fixture. The fresh container
and synthetic runtime state were removed after evidence capture. See the
curated record at
[`evidence/crew-50-uid-soak-20260922.md`](evidence/crew-50-uid-soak-20260922.md).

## Native CLI refresh and Crew self-test coverage (2026-09-22)

The native macOS arm64 CLI and daemon were refreshed in the bounded shared
target with `CARGO_BUILD_JOBS=2` and `CARGO_INCREMENTAL=0`:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 \
  cargo build --locked -p biorouter-cli --bin biorouter \
  -p biorouter-server --bin biorouterd
```

These outputs are native Mach-O arm64 binaries for the macOS refresh, not
Linux acceptance evidence:

```text
/private/tmp/biorouter-crew-target/debug/biorouter
  SHA-256 784c945022ca661b2cb97c8677686c044061226e00a9fa26bc752b0aa5b2271c
/private/tmp/biorouter-crew-target/debug/biorouterd
  SHA-256 5aa6d1c3e539e3c3c635ef0660af599aba51a008f49fa44c12a7c29937d2ded3
```

With isolated writable `HOME` and `BIOROUTER_PATH_ROOT`, `biorouter
--version` printed `1.91.1` and `biorouter crew --help` exited 0. The default
path smoke attempt hit a log-file permission boundary and was not treated as
a pass.

The self-test workflow now covers Crew CLI/daemon parity, SHA-verified text
transfer receipts and owned-partial cleanup, duplicate receipt idempotency,
and no-proof refusal cases for symlink/capability and unknown-transfer
boundaries. Its focused workflow isolation test passed 4/4. The workflow does
not acquire human proof, first-host approval, credentials, or provider access.

The subsequent native clippy run reached a source compile blocker in the
latest daemon stop test change: `crates/biorouter-cli/src/daemon_client.rs:796`
spawns `wait_for_daemon_stop(&expected_descriptor(&directory))`, leaving a
temporary borrowed value in a `'static` task (`E0716`).

## Actual isolated self-test workflow run (2026-09-22)

The required workflow was launched once with a fresh isolated root and no live
profile access:

```text
timeout 600 env \
  HOME=/private/tmp/crew-selftest-actual-luna \
  BIOROUTER_PATH_ROOT=/private/tmp/crew-selftest-actual-luna \
  BIOROUTER_PROVIDER=ollama BIOROUTER_MODEL=qwen3:8b \
  biorouter run --workflow biorouter-self-test.yaml \
  --params test_phases=computer_use --params test_depth=quick \
  --params parallel_tests=false --params cleanup_after=true \
  --params workspace_dir=/private/tmp/crew-selftest-actual-luna/workspace \
  --output-format stream-json --max-turns 10 --debug
```

The run used the approved local Ollama `qwen3:8b` model, session
`20260922_1`, and did reach the actual workflow agent. It created only the
isolated profile state under `/private/tmp/crew-selftest-actual-luna`; no
human proof, first-host approval, secret, live profile, or fixture was
entered. The agent confirmed no GUI workspace was attached.

The run terminated after about two minutes with provider failure:
`Ollama completed without answer text or tool calls. The model may have
returned reasoning only.` The CLI explicitly advised that retrying would not
help until the cause changes. This is a provider failure, not a passing or
skipped workflow result; no Crew acceptance assertions were observed. The
isolated state was retained for inspection and was not used against any live
profile.

## Full desktop Vitest run (2026-09-22)

The bounded full desktop suite was run once from `ui/desktop` with the
repository test script and two workers:

```text
npm run test:run -- --maxWorkers=2
```

Vitest was `4.0.18` (`test:run` is `vitest run`). The result was **547 test
files passed**, **6,242 tests passed**, **19 skipped**, and **0 failed**
(6,261 total), in `181.80s`. The run produced existing non-failing React
`act(...)`, jsdom canvas, and mocked-network stderr diagnostics; no test
failed. Captured output is at
`/private/tmp/crew-ui-test-run-luna.log`.

The uncommitted source scope was recorded as 27 existing `ui/desktop` tracked
or untracked files with manifest SHA-256
`5f2473a9d64205a684c95fbec795d84204f5d940581ca71d872318634fbc3fab`. This
entry records the concurrent working-tree scope; the test command made no
source edits.

## Final repository gate (2026-09-22)

Against commit `8d6c2ae4` plus the owned documentation/evidence changes, the
required command passed end to end:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 just check-everything
```

All ten checks passed: Rust formatting; production and baseline clippy; the
banned-TLS scan; non-inheritable production socket scan across 17 targets; UI
lint/typecheck, themes, 404 contrast assertions, and token mirrors; OpenAPI
generation/schema drift; version, brand, and Biorouter Copilot naming;
vendored source; cross-drift; registry (61/61 tests, 38 extensions and 129
skills current); and privacy registry (21/21 tests, all copies agreeing).
`git diff --check` passed afterward. No live fixture or profile was changed by
this gate.

## Linux ARM64 artifact and ordinary-user lifecycle qualification (2026-09-22)

The ARM64 Linux CLI and daemon were built from read-only source in the
immutable cached image `rust@sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`,
using `/private/tmp/crew-linux-cli-daemon-target-luna`, Cargo jobs 2, and
incremental compilation disabled. The build began at HEAD
`b90939b73ffa4d12f2dd4799ac499f4a3a22060d`, a descendant of
`8d6c2ae4`; Crew transfer source and tests changed during the seven-minute
build, so these artifacts are qualified as timing-dependent and are not final
post-fix provenance.

The resulting ELF ARM64 artifacts were:

```text
/private/tmp/crew-linux-cli-daemon-target-luna/aarch64-unknown-linux-gnu/debug/biorouter
  SHA-256 06eaccfa5c2345cf5c4301753cff8c78dcc6ceb170e80636e479f4eea901e635
  BuildID c2920a9ad419cf054f873a177363d39df20a6ee6
/private/tmp/crew-linux-cli-daemon-target-luna/aarch64-unknown-linux-gnu/debug/biorouterd
  SHA-256 1be2c91da9dd59246e930655650c09b883e1fede895b9e7ac8dc2d2873e70f4b
  BuildID ead6db1359d217991bc031a35e5027e4f1330ccf
```

Both imported `GLIBC_2.39` as their highest GLIBC symbol. In the immutable
image, the CLI printed `1.91.1`, the daemon printed
`biorouter-server 1.91.1`, and `crew --help` exited 0.

A separate disposable container ran the CLI as ordinary UID 1101 with a
fresh profile owned by that user. Synthetic daemon start, status, stop, and
restart all passed; the profile ID remained stable across restart and the
instance ID changed. A wrong-format proof was rejected, and a valid-format
but incorrect proof reached the daemon and was rejected with 403. Credential
unlock remained uninitialized and correctly refused the synthetic action.
The disposable container was removed; no retained CLI workspace, live
profile, or SSH fixture was touched. This is headless Linux lifecycle
evidence only and provides no GUI acceptance evidence.

## Final committed native CLI and daemon build (2026-09-22)

From committed HEAD `8d6c2ae4f30da1095f94cbe1b2cc79bdb521e1fe`, the final native
macOS arm64 artifacts were rebuilt with the bounded shared target and two
Cargo jobs:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 \
  cargo build --locked -p biorouter-cli --bin biorouter \
  -p biorouter-server --bin biorouterd
```

`biorouter` is Mach-O arm64, SHA-256
`f27849c0c3224db9365e25090e360f9dc3f06acc1da9c86ce66d85bb92c41e32`.
`biorouterd` is Mach-O arm64, SHA-256
`bcfd9f0bc27b3f6d74b8027fc86958616d44d7df165a34b6b29aef13a33e09d3`.
With isolated writable `HOME` and `BIOROUTER_PATH_ROOT`, the CLI printed
`1.91.1`, the daemon printed `biorouter-server 1.91.1`, and `crew --help`
exited 0. No live process or fixture was restarted or modified.

### Native Crew CLI MFA/ProxyJump acceptance (2026-09-22)

A separate disposable profile and synthetic Linux PAM fixture were exercised with refreshed native CLI/daemon artifacts (CLI SHA-256 `784c945022ca661b2cb97c8677686c044061226e00a9fa26bc752b0aa5b2271c`; daemon SHA-256 `5aa6d1c3e539e3c3c635ef0660af599aba51a008f49fa44c12a7c29937d2ded3`). The native `biorouter crew auth` path succeeded through a strict two-hop localhost ProxyJump using an encrypted synthetic key and final keyboard-interactive PAM; `authenticated: true`, exit code 0. Daemon status remained connected after CLI exit. Wrong-secret rejection, prompt cancellation, PTY resize recovery, and file/log secret sweeps passed. This is synthetic fixture evidence only and does not claim institutional MFA, Duo/TOTP, or production identity behavior.


## Consolidated Crew-focused Rust filters (2026-09-22)

These filters were run against HEAD `8d6c2ae4` with the shared bounded target
`/private/tmp/biorouter-crew-target`, `CARGO_INCREMENTAL=0`, and
`CARGO_BUILD_JOBS=2`. They were run sequentially; the counts below are
independent of earlier narrower authentication, transfer, and stop tests.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --lib 'crew::' -- --nocapture
```

Result: **37 passed, 0 failed, 4,179 filtered out**.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter-server --lib 'crew' -- --nocapture
```

Result: **18 passed, 0 failed, 712 filtered out**.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter-cli --lib 'commands::crew' -- --nocapture
```

Result: **3 passed, 0 failed, 489 filtered out**. The CLI filter selected
three real command-output/input tests; it was not a zero-test filter.

At the time of this entry, `git rev-parse --short HEAD` returned `8d6c2ae4`,
and `git status --short` contained only documentation and evidence paths under
`docs/research/biorouter-crew`; no product source paths were dirty. The earlier
focused authentication (5), transfer (4), filesystem (9), and stop (3) counts
are retained as separate evidence and are not added to these consolidated
filter totals.

## Final exact-commit Linux archive and lifecycle (2026-09-22)

An isolated archive was created from exact commit
`37f2684af6d500c27564a1c4351f84da985ec924`; archive SHA-256:
`b2b5b74ca6f95ec6716a980ab18ac3f5eff3c029e988f913d242ebbdd196ab27`.
Only that extracted archive was mounted read-only into the immutable ARM64
Docker image `rust@sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`.
The build used a separate target, Cargo jobs 2, and incremental compilation
disabled.

The final Linux artifacts were native ARM64 ELF PIE binaries:

```text
/private/tmp/crew-linux-final-37f-target/aarch64-unknown-linux-gnu/debug/biorouter
  SHA-256 2df5191c33611e442fe5a6ab9971ac7b1fc7d1ae36a1975f11c6ee90bfdc63d7
  BuildID 18d95c10639ea59e104c72b6035980096a4f468a
/private/tmp/crew-linux-final-37f-target/aarch64-unknown-linux-gnu/debug/biorouterd
  SHA-256 1ce98597931b3e37000719977de29fac4224b7a7f7c11bb6a565df2a6a157e6a
  BuildID e26d6229cc2c731416f0bd9ae8309ada5b5c7db7
```

Both imported `GLIBC_2.39` as their highest GLIBC symbol. In the immutable
image, the CLI printed `1.91.1`, the daemon printed
`biorouter-server 1.91.1`, and `crew --help` exited 0.

An ordinary UID 1101 disposable profile smoke passed daemon start/status,
stop, and restart. The profile ID remained stable while the instance ID
changed after restart. A valid-format incorrect proof was rejected with 403;
the final stop succeeded. No retained workspace, live profile, or SSH fixture
was touched, and the disposable container was removed. This is headless Linux
lifecycle evidence and does not provide GUI acceptance evidence.

## Crew-only self-test workflow gate (2026-09-22)

The workflow exposes an explicit `test_phases=crew` phase and includes it in
`test_phases=all`. The Crew-only rendered `Workflow.extensions` roster is
`developer` plus platform `crew`; unrelated heavy extensions
(`computercontroller`, `webdocuments`, `agent_drafter`, `autovisualiser`,
`code_execution`, and `skills`) are excluded in that phase. The `all`
roster retains the existing UI extensions and adds platform `crew`. The
integration test asserts the parsed roster, not prompt text.

Pre-run checks on the final source passed:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --test self_test_workflow_isolation -- --nocapture
```

Result: **5 passed, 0 failed, 0 filtered out**.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo clippy -p biorouter --test self_test_workflow_isolation -- -D warnings
```

Result: strict Clippy passed. `cargo fmt --all -- --check` and
`git diff --check` also passed. Current source hashes are
`biorouter-self-test.yaml`
`36a9e41a622305207bd827e9e299af218bf96316700d136f456801c9c090f6fa` and
`crates/biorouter/tests/self_test_workflow_isolation.rs`
`1a7a9c1218dc846aced6499baddb6ed469cc142c198f4db3c7d00ca1f2689501`.

Historical model runs before the final roster fix remain **NOT PASS**: the
initial run lacked platform `crew`, and subsequent qwen3:1.7b/qwen3:8b
attempts produced no Crew request result or ended in provider/action-limit
failure. Their logs remain under `/private/tmp/crew-selftest-crew-0450-*`.

The one corrected real run used the lane-owned Ollama loopback
`127.0.0.1:11435`, already-installed qwen3:8b, fresh synthetic profile
`/private/tmp/crew-selftest-crew-roster-luna`, explicit
`--with-builtin developer,crew`, and `BIOROUTER_MAX_TOKENS=8192`. Its actual
`workspace__workspace_list` response reported exactly `crew` and
`developer`; `developer__shell` and `developer__shell_status` returned
real tool results. The phase helper's exact shell request ran `which biorouter
&& which biorouterd` after printing only its BIOROUTER variables. It resolved
`biorouter` to `/Users/wgu/.local/bin/biorouter` and reported
`biorouterd not found`, then exited non-zero. The pinned native 906bf pair
was available separately at
`/private/tmp/biorouter-crew-artifacts-906bf68/{biorouter,biorouterd}` with
SHA-256 values `4095d6e2b902be43273535de61fd3eb323a757eb1c49b8470cf67e61ff7437a8`
and `45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9`;
that directory was not prepended to the model shell PATH. The run therefore
did not verify the pinned pair, produced no native CLI help/version result,
and produced no `crew__request` result. It stopped at the configured
10-action cap after an out-of-scope empty worktree directory attempt; that
exact empty artifact was removed. Runtime Crew acceptance remains **NOT PASS**.
Evidence: `/private/tmp/crew-selftest-crew-roster-luna.stream.jsonl`.

The final PATH-pinned retry used unchanged `HOME`, a fresh profile at
`/private/tmp/crew-selftest-crew-pinned-luna`, the lane-owned Ollama loopback
on `127.0.0.1:11435`, and `biorouter run` resolved through
`PATH=/private/tmp/biorouter-crew-artifacts-906bf68:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin`.
Before launch, `command -v` resolved both binaries to the pinned directory;
`biorouter --version` printed `1.91.1` and `biorouterd --version` printed
`biorouter-server 1.91.1`. During the model run, actual shell output again
showed both pinned paths. The model then attempted `curl` against nonexistent
`localhost:8080` and received connection failure; no `crew__request` result,
four native help/version tool results, or phase-owned mutation audit was
produced. The workspace was cleaned by the model, and the run was stopped at
the bounded action limit. Runtime Crew acceptance remains **NOT PASS**.
Evidence: `/private/tmp/crew-selftest-crew-pinned-luna.stream.jsonl`.

## Current privacy-mode and observer focused regressions (2026-09-22)

These focused filters were run with the bounded shared target
`/private/tmp/biorouter-crew-target`, `CARGO_INCREMENTAL=0`, and
`CARGO_BUILD_JOBS=2`. Counts are independent of the earlier consolidated
filters above.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --lib crew::tests -- --nocapture
```

Result: **17 passed, 0 failed, 4,201 filtered out**. This includes the
message/blob privacy-mode mismatch guards and run-admission origin/mode
guards, with matching and omitted mode fields exercising the compatible
paths.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter-server --lib routes::crew_observation::tests -- --nocapture
```

Result: **6 passed, 0 failed, 731 filtered out**. The observer state wire
contract test covers valid public/private mode and connection identity fields,
plus missing and invalid mode rejection.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter-server --lib crew::transfers::transfers_tests -- --nocapture
```

Result: **7 passed, 0 failed, 732 filtered out**. These tests cover legacy
`FileRequest` decoding, expected-mode round trips, and receipt-bound cleanup
after the live connection has been removed.

Strict focused lint passed for both affected crates:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo clippy -p biorouter --lib --tests -- -D warnings
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo clippy -p biorouter-server --lib --tests -- -D warnings
```

`just generate-openapi` completed successfully, and affected-file
`rustfmt --check` plus `git diff --check` passed.

At this focused-test checkpoint, live transfer policy checks were still open.
They subsequently passed on the pinned `906bf68b` pair: mismatched mode was
refused before selection of a nonexistent file, a matching 43-byte upload
completed with its expected digest, and an opaque capability issued in private
mode was refused after a saved-mode change with no transfer receipt created.
See the [current CLI evidence](evidence/crew-cli-observer-context-20260922.md).
These live API/CLI cases do not establish graphical picker acceptance.

## Final exact-commit 906bf Linux archive and lifecycle (2026-09-22)

The final Linux validation used exact source commit
`906bf68b56b770e700159b2fd021c99f6e53df5e`, archived to
`/private/tmp/crew-linux-final-906bf-source.tar` with SHA256
`30e311c92099edcb3e37be4195b52d51cb84824682f1541e38a3a8a7899ca157`.
The archive was mounted read-only into immutable Docker image
`rust@sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`
for `linux/arm64`; the command used `cargo build --locked -p biorouter-cli
--bin biorouter -p biorouter-server --bin biorouterd
--target aarch64-unknown-linux-gnu`, with `CARGO_BUILD_JOBS=2` and
`CARGO_INCREMENTAL=0`. The existing isolated target directory was
`/private/tmp/crew-linux-final-37f-target`; no live fixture was used.

The resulting ELF artifacts were:

* `biorouter`: ARM aarch64 PIE, BuildID
  `4b378c0326c264bdc8a709c9f26c4dcf8f4cb411`, SHA256
  `ff3bfb9ba5a872912c806a152ef57e69761984411ea881446a36ff0cae53c58f`.
* `biorouterd`: ARM aarch64 ELF, BuildID
  `19300c3484557243d66a67f94842b493b6fae563`, SHA256
  `3c6049913688cb7d72b7b41874ac2d833411b51693a0bef5dfcb0859f9041bf0`.

`readelf --version-info` reported `GLIBC_2.39` as the highest imported GLIBC
symbol for both artifacts. Immutable-container checks reported `biorouter`
version `1.91.1` and `biorouterd` version `1.91.1`; CLI help accepted both
`--expected-mode private` and `--expected-mode public` forms.

An isolated disposable ARM64 container ran the ordinary UID 1101 lifecycle
against a fresh mode-700 profile. Start, status, stop, restart, status, and
final stop all returned the expected results. The profile ID remained
`3d4542bf-9494-44cd-b7f7-263773bd9016` across restart while the instance ID
changed from `fa4755b9-460a-44cd-b7bc-c058aaa57758` to
`fdaa7d39-83e5-4ea8-bd2a-3eec221a9cf4`. A wrong proof was rejected with HTTP
403 and the stopped status returned the expected missing-daemon refusal. The
disposable container was removed; retained fixtures, profiles, keys, and
workspace state were not touched. This is headless Linux evidence and does
not claim GUI parity.

## Final committed native pair and full repository gate (2026-09-22)

The native macOS debug artifacts were built from committed source
`906bf68b56b770e700159b2fd021c99f6e53df5e` with the bounded shared target,
incremental compilation disabled, and two Cargo workers:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo build -p biorouter-cli -p biorouter-server
```

Cargo completed successfully in 2m01s. The pinned copies are in
`/private/tmp/biorouter-crew-artifacts-906bf68/`:

* `biorouter` (Mach-O arm64), SHA256
  `4095d6e2b902be43273535de61fd3eb323a757eb1c49b8470cf67e61ff7437a8`.
* `biorouterd` (Mach-O arm64), SHA256
  `45d42240af92ce4a0d43618bd4d496249f095fcdad88af768ad9f2ecd086baa9`.

The required full gate was then run against the same commit and bounded
target:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  just check-everything
```

The command exited 0. The terminal output was captured by execution session
`61060`; no separate persistent log file was created and total elapsed time
was not recorded. Formatting, all clippy gates (including the inheritable
socket gate), UI typecheck/lint/theme/contrast/token checks, OpenAPI
regeneration/schema comparison, version, brand, Copilot naming, vendored
source, cross-drift, registry (**61 passed**), and privacy-registry
(**21 passed**) checks all passed. No additional source or generated-file
changes were introduced by the gate.

## Final desktop Vitest gate after Crew observer and upload regressions (2026-09-22)

Against committed source `906bf68b56b770e700159b2fd021c99f6e53df5e`, the final
desktop suite used two workers:

```text
npm run test:run -- --maxWorkers=2 --reporter=dot
```

Result: **550 test files passed; 6,256 passed, 19 skipped, 0 failed** (6,275
tests total). Vitest duration was **161.42s** and `/usr/bin/time -p` reported
**real 161.85s**. Full output is retained at
`/private/tmp/crew-ui-vitest-final-906.log`. This run includes the Crew
observer recovery and upload/picker privacy regressions; expected existing
test warnings and skipped Playwright checks were non-failing.

## G13 filesystem authority regressions (2026-09-22)

Added the Unix-only integration target
`crates/biorouter-server/tests/crew_transfer_authority.rs`. Its temporary
roots use `/private/tmp` on macOS, where the production no-follow directory
walk intentionally rejects the `/var` symlink chain, and the platform default
temporary root elsewhere; no live profile or credential state is used.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter-server --test crew_transfer_authority -- --nocapture
```

Result: **4 passed, 0 failed**. Strict clippy also passed:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo clippy -p biorouter-server --test crew_transfer_authority -- -D warnings
```

The proven G13 subset is bounded to: source selection remaining anchored to
the originally opened inode after path replacement; publication remaining on
the originally selected directory after directory inode replacement; raced
target creation refusing implicit overwrite while preserving the existing
sentinel and owned partial; symlink substitution refusal; and explicit
hardlink replacement preserving the unrelated hardlink sentinel and removing
only the owned partial after successful publication. This does not claim
whole-transfer daemon, Windows, or GUI coverage.

### G13 focused follow-up after target-approval handoff

The prior 4-test count above is superseded by the current bounded run. The
Unix authority integration target now reports **6 passed, 0 failed, 0
ignored** with the same bounded command. The expanded cases cover absent to
created targets, approved existing-target replacement, and a held original
partial file whose named path is replaced before final publication.

The focused private transfer module reports **11 passed, 0 failed, 734
filtered out**:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter-server --lib crew::transfers::transfers_tests -- --nocapture
```

This includes start and resume pending-capability gates, expiry without TTL
renewal, discard quota/receipt preservation, publication recovery, malformed
receipt rejection, and the completed-replay helper preserving published bytes
while restoring the original selection approval. The replay case is a
registration/bind helper regression; it is not a manager-backed end-to-end
start claim.

The route proof-gating module reports **3 passed, 0 failed, 742 filtered out**
for confirm/discard requests without human proof, including API-key-only
requests. The existing filesystem target reports **9 passed, 0 failed, 0
ignored** after its temporary-root helper was made portable through the
canonical system temporary directory.

The strict server gate passed after removing three redundant test-only borrows:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo clippy -p biorouter-server --lib --tests -- -D warnings
```

`just generate-openapi` then completed successfully and regenerated both
`ui/desktop/openapi.json` and the frontend API bindings. Rustfmt checks for the
four touched test modules and `git diff --check` both passed. These tests do
not claim a manager-backed live `TransferService::confirm` run with a changed
target; that remains an explicit integration gap. They also do not claim
Windows or GUI parity.

## Crew transport sticky-failure tests (2026-09-22)

The focused Unix-local transport suite ran against the shared bounded target;
all peers were synthetic `/bin/sh` processes with temporary marker files, so
the run used no real SSH, credentials, profiles, or network endpoints:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --lib crew::transport::tests -- --nocapture
```

Result: **9 passed, 0 failed, 0 ignored, 4,218 filtered out** in **0.26s**.
The cases cover valid-response rearming, structured denial recovery, local
oversize rejection before poisoning, mismatched-ID no-retry/no-second-write,
EOF, incomplete and malformed frames, cancellation after write, and paired
stale/current `Arc` retirement behavior. The matching retirement assertion
also checks disconnected status, sanitized recovery guidance, unchanged
connection mode, and exact peer-child exit.

Strict Clippy passed with:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo clippy -p biorouter --lib --tests -- -D warnings
```

`cargo fmt --all -- --check` and `git diff --check` passed. The source inputs
for this validation were commit `67a32a075100af8f348e4c86b208b21848d21450`
with transport handoff hash
`058e966781cc6675fe341a0dba974f2707df5f5ce7a5d1aa6d5255020dd2db02` and test
hash `109a359b545e912dcd1b2af239c175019a3c006130d12ba91e219f54302c5e31`.

## Current full gate and native artifact handoff (2026-09-22)

After the transport change was committed as product source `9a3957d6`, the
current documentation/evidence source was `d3498ecdf55a43976d6ef123faae0a5872a428de`.
The current full repository gate completed successfully with the shared
bounded target:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  just check-everything
```

The gate passed Rust formatting and clippy, including **17 production
targets** with zero ordinary warnings and the non-inheritable-socket check;
UI typecheck/lint/theme/contrast/token checks; OpenAPI generation and schema
comparison; version, brand, Copilot naming, vendored-source, and cross-drift
checks; registry (**61 passed**) and privacy-registry (**21 passed**) checks.
The command was run in execution session `63648`; no persistent log file was
created by the gate.

Using the successful native build outputs from the same current source, the
CLI and daemon were built with:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo build --bin biorouter --bin biorouterd
```

The build completed successfully in execution session `65100` (`Finished
dev profile` after 2m18s). The immutable handoff directory is
`/private/tmp/biorouter-crew-artifacts-9a3957d6/`:

* `biorouter` is arm64 Mach-O, version **1.91.1**, SHA256
  `9bbedb34c349e3b637c8b8e01e6e4e50807ffa86bf1bcfaa1562fc3c64195c5f`.
* `biorouterd` is arm64 Mach-O, version **1.91.1**, SHA256
  `50eefaaebad298679d9ad525a98a911941ea8ff62c462033a389fcd3351b005f`.

Both files have mode `0555`. This is a native macOS artifact handoff; it
does not claim Linux, Windows, GUI, or hosted runtime acceptance.

## Full desktop Vitest validation (2026-09-22)

The full desktop suite ran once against product source commit `9a3957d6` and
documentation/evidence source commit `d3498ecdf55a43976d6ef123faae0a5872a428de`:

```text
npm run test:run -- --maxWorkers=2 --reporter=dot
```

Result: **550 test files passed, 6,260 tests passed, 19 skipped, 0 failed**
(6,279 total) in **160.46s wall time** (`real 160.46`, `user 289.09`,
`sys 47.99`). The complete captured output is preserved at
`/private/tmp/crew-ui-vitest-d3498ecd.log`. This is a local desktop Vitest
run; it does not claim native GUI, daemon, Linux, Windows, or hosted-runtime
acceptance.

## Transport diagnostic classification follow-up (2026-09-22)

Astra's transport handoff added stable sanitized `ssh_*` failure categories
and captures only the child exit status observed before cleanup. The focused
Unix-local suite uses synthetic shell peers and temporary markers only; it
does not use real SSH, credentials, profiles, or network endpoints.

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --lib crew::transport::tests -- --nocapture
```

Result: **12 passed, 0 failed, 0 ignored, 4,218 filtered out** in **0.27s**.
The natural-exit case proves `ssh_write_io_broken_pipe` with
`child_before_cleanup=exit_23`; EOF and incomplete-frame cases assert stable
categories without scheduler-dependent exit timing. Diagnostics also verify
that wire/request sentinel values are absent from errors.

The broader Crew library subset passed with:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo test -p biorouter --lib 'crew::' -- --nocapture
```

Result: **51 passed, 0 failed, 0 ignored, 4,179 filtered out** in **20.90s**.
Strict Clippy passed with `-D warnings`:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo clippy -p biorouter --lib --tests -- -D warnings
```

`cargo fmt --all -- --check` and `git diff --check` passed. These results
were committed with the transport diagnostic tests as `4a2e190b`.

## Final current-source gate and native artifact handoff (2026-09-22)

The final current-source gate completed successfully before the native build:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  just check-everything
```

It passed Rust formatting and clippy, including **17 production targets** with
zero ordinary warnings and the non-inheritable-socket check; UI typecheck,
lint, themes, contrast (**404 assertions**), and tokens; OpenAPI generation
and schema comparison; version, brand, Copilot naming, vendored-source, and
cross-drift checks; registry (**61 passed**) and privacy registry (**21
passed**). No gate failures were reported.

The final native pair then built successfully from the same source:

```text
source bin/activate-hermit && \
  CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target \
  CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 \
  cargo build --bin biorouter --bin biorouterd
```

The build finished in **2m02s**. Immutable artifacts are available at
`/private/tmp/biorouter-crew-artifacts-4a2e190b/`:

* `biorouter`: arm64 Mach-O, version **1.91.1**, mode `0555`, SHA256
  `0c1659478c1de91794170e311770abb6e20663b3a358cab68326f76ef4a6429e`.
* `biorouterd`: arm64 Mach-O, version **1.91.1**, mode `0555`, SHA256
  `41ccbf13d4cc6088c340204a32394234f8f0ff7bec45601f9f19fa236ade6203`.

The source revision is `4a2e190b2480d08fd04799d60081c316ff791973`.
This is a native macOS artifact handoff and does not claim Linux, Windows,
GUI, or hosted runtime acceptance.

## Exact-commit Linux ARM64 validation and UID 1101 lifecycle (2026-09-22)

An immutable archive was created from exact commit
`4a2e190b2480d08fd04799d60081c316ff791973`; its SHA-256 is
`f71d8df325bbacb658dd20b18855a0c022564595b5335d57fb0842b38b3fecc5` and the
exact committed `Cargo.lock` SHA-256 is
`60da88634d074791ee19ad2bd1c9000054166d8e391c4c027151982d4976d03e`. The
archive was mounted read-only into pinned ARM64 Linux image
`rust@sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082`.
The locked build used `CARGO_BUILD_JOBS=2` and `CARGO_INCREMENTAL=0` and
completed successfully. The resulting ELF artifacts were:

```text
biorouter  SHA-256 c83fb5597f133a3d29dd98802c26738dd325fa83d0f528a153037fb1bc39f5a6
biorouterd SHA-256 c8c487d569ebe975bbf0825c04c654ee164337789fd2a5623b7f452401943734
```

Both are native ARM64 ELF PIE binaries; `readelf --version-info` reports
`GLIBC_2.39` as the highest imported GLIBC symbol for each. The CLI and daemon
reported version `1.91.1`, and `biorouter crew --help` exited successfully.

Focused tests from the same archive passed sequentially: the transfer unit
filter selected **11 passed, 0 failed, 734 filtered**; the route proof filter
selected **3 passed, 0 failed, 742 filtered**; and
`--test crew_transfer_filesystem` selected **8 passed, 0 failed**. The exact
archive contains eight filesystem tests; earlier documentation claiming nine
does not match this source snapshot.

A fresh disposable ARM64 container ran the built pair as UID 1101 with a new
tmpfs profile root and freshly generated synthetic approval and encrypted-vault
secrets. The lifecycle container used the top-level daemon path after a later
Cargo test invocation had replaced it with test-feature variant SHA-256
`3d4ed8b07e0c64896aec59a99643cb0bd9418efb69c3f4ebba42f10abfa26c8d`; it did
not use the verified build daemon SHA-256 above. Therefore this lifecycle is
an unqualified daemon smoke and does not qualify the verified build pair.
Daemon start/status passed with profile ID
`89d43292-04eb-45e0-833a-7a80e9ebe3e4` and instance ID
`2e49cdfa-f3f3-4911-9fc1-f70ebe6bd7ae`; the encrypted vault initialized and
locked successfully. A valid-format incorrect approval proof returned HTTP
403. Stop succeeded, restart preserved the profile ID and changed the instance
ID to `2214cf7f-03ea-4896-bacc-935e00552ee1`, and the final stop succeeded.
Final daemon status returned the expected missing-daemon refusal. The container
and its tmpfs profile were removed on exit; no existing fixture, profile,
workspace, AWS resource, or native CUA surface was accessed.

The verified daemon build artifact was recovered from the immutable build
target's dependency output at
`/private/tmp/crew-linux-final-37-f-target/debug/deps/biorouterd-57bde4174d3ee770`,
SHA-256 `c8c487d569ebe975bbf0825c04c654ee164337789fd2a5623b7f452401943734`.
It has the original build BuildID and was preserved alongside the verified CLI
in `/private/tmp/crew-linux-verified-4a2e190b.zlzUET/`, both mode `0555`. The
top-level `debug/biorouterd` and `debug/deps/biorouterd-bd8214378db64a94`
are test-feature variants and were excluded from verified artifact claims.

### Verified-pair UID 1101 lifecycle rerun

The lifecycle was repeated in a new disposable ARM64 container as UID 1101
with the preserved mode-0555 pair above mounted read-only. SHA-256 checks were
run immediately before both daemon launches and matched the verified CLI and
daemon hashes. Fresh synthetic approval and encrypted-vault secrets were used.
Start/status passed with profile ID `670bf2be-29ee-4ee2-b5cf-7883713d8baa`
and instance ID `28087f97-6fb2-4bff-9652-3052ed3d1146`; vault initialization
and locking passed; a valid-format wrong proof returned HTTP 403. After stop,
restart preserved the profile ID and changed the instance ID to
`1d416749-0b9e-4067-bf5d-2c4212151996`; the final stop passed and final status
returned the expected missing-daemon refusal. The disposable container and
tmpfs profile were removed on exit. This is verified-pair headless Linux
lifecycle evidence only; it does not establish GUI or cross-platform parity.

### Fresh three-user SSH fixture bootstrap

Using the pinned native 4a pair (CLI SHA-256
`0c1659478c1de91794170e311770abb6e20663b3a358cab68326f76ef4a6429e`; daemon
SHA-256 `41ccbf13d4cc6088c340204a32394234f8f0ff7bec45601f9f19fa236ade6203`)
and the verified Linux broker SHA-256
`4f11d8586b093a6616f0d8231930112369e990165e1adfce88b6a2bdb031351a`, a fresh
Ubuntu 24.04 ARM64 SSH fixture was prepared with three distinct Unix accounts
and three distinct native profiles. Strict host-key checking and the broker's
supported per-user `~/.local/bin/biorouter-crew` path were used.

All three fresh devices authenticated and connected to the same private
workspace; Bob and Carol enrollment succeeded. A team, `public-safe` General
channel, and restricted Private channel were created, with all three
principals invited and accepted. An Alice message and Bob reply were both
observed by Carol through channel history. This evidence covers fixture
enrollment, channel membership, and human-message roundtrip only. It does not
claim new shared-conversation behavior, GUI/native acceptance, AWS export, or
real-model validation.

### Shared daemon conversation and durable elicitation checkpoint

GPT-5.6 Luna authored and executed these checks on the conversation refactor
after `4a2e190b`. GPT-6 Astra authored the production changes and independently
reviewed the adapter, persistence order and safe SSH diagnostics. The following
are separate, overlapping test scopes, not an aggregate test count:

| Command | Observed result |
| --- | --- |
| `cargo test -p biorouter-cli daemon_client::tests:: -- --nocapture` | 16 passed, 0 failed; before the later SSH diagnostic display change. |
| `cargo test -p biorouter-cli 'cli_tests::shared' -- --nocapture` | 4 passed, 0 failed. |
| `cargo test -p biorouter-cli commands::shared_conversation::tests -- --nocapture` | 5 passed, 0 failed. |
| `cargo test -p biorouter-server --bin biorouterd new_session_provider_binding_tests -- --nocapture` | 11 passed, 0 failed. |
| `cargo test -p biorouter-server --bin biorouterd elicitation_tests -- --nocapture` | Final strengthened suite: 4 passed, 0 failed. |
| `npm run test:run -- src/components/ElicitationRequest.test.tsx` | 2 passed, 0 failed. |
| `npm run test:run -- src/hooks/chatStreamStore.userAction.test.tsx` | 6 passed, 0 failed. |
| `npm run typecheck` | Passed after the response-object narrowing correction. |

The elicitation suite verifies the persisted answer ID/body and agent-only
visibility, `Message` then `MessagesPersisted` event order, no extra rows from
foreign-session or replay attempts, a persisted user-only cancellation receipt
with no answer, and HTTP 500 `persistence_failed` without answer delivery or
success events when persistence is refused.

The new core diagnostic test
`terminal_failure_guidance_is_allowlisted_and_never_echoes_unknown_codes`
selected one test and passed. Its encompassing command was stopped while Cargo
traversed unrelated zero-match integration binaries; a successful command exit
is not claimed. The first elicitation filter also selected zero tests and was
corrected before reporting the selected suite above.

`just generate-openapi` regenerated the schema and frontend bindings, including
the 500 response. The first UI typecheck correctly failed TS2339 on accessing an
unknown response's `status`; Astra added object narrowing, and Luna's repeat
passed. `cargo fmt --all` ran immediately before the final elicitation test,
and `git diff --check` passed after the source files were released.

This checkpoint does not establish the final full gate, a production binary
build, ordinary CLI model/MCP behavior, current mixed GUI/CLI acceptance or AWS
product testing. The fresh three-user fixture above still uses the earlier
pinned native pair and awaits the new production artifacts.

### Clean-head gate and focused UI follow-up

On clean HEAD `74190f6b6e6002b62104aa4cf549564149e5ed99`,
`just check-everything` passed. The bounded core regressions also passed:
`cargo test -p biorouter --lib action_required_manager` selected 7 tests,
and `cargo test -p biorouter --lib pending_user_action` selected 30 tests;
all passed with zero failures.

The first full desktop run (`npm run test:run -- --maxWorkers=2`) selected
551 files and 6,281 tests: 550 files passed and 1 failed; 6,261 tests passed,
19 were skipped, and one failed in
`src/components/crew/CrewView.regression.test.tsx:283`. The reconnect test
observed the transient empty snapshot from `refresh()` before the replacement
observer frame had run, then clicked the disabled reconnect button. The test
now waits for the replacement observer call and preserves the same empty-draft
assertion. The bounded rerun
`npm run test:run -- --maxWorkers=2 src/components/crew/CrewView.regression.test.tsx`
passed 9/9; its captured pre-fix log is `/tmp/crewview-regression-luna.log`.

The requested server route filter was attempted both in parallel and with
`--test-threads=1`. The parallel run stalled with five tests over 60 seconds;
the serial retry completed the elicitation cases and eight route cases before
stalling at `secrets_tests::an_empty_required_field_leaves_the_dialog_open`.
Both processes were stopped; no full route-filter pass is claimed.

No production build or immutable artifact copy was performed after this gate.

After correcting the reconnect test to capture the pre-refresh observer count
and await an enabled reconnect control, the bounded focused file passed 9/9.
A subsequent full desktop run with `npm run test:run -- --maxWorkers=2` passed
551 files and 6,262 tests, with 19 skipped and zero failures. The captured log
is `/tmp/desktop-vitest-luna-final.log`.

### Final-head bounded native checks

At clean source HEAD `a11ea7e9`, the focused extension-install suite passed 31
tests and the serialized server action-required route filter passed all 16
tests, including the four elicitation tests and the secret-dialog cases that
had previously contended when run in parallel. The subsequent
`just check-everything` run stopped in clippy before the later checks: Rust
clippy reported `clippy::nonminimal_bool` twice for the expression at
`crates/biorouter/src/extension_install/transaction.rs:956`, and the recipe
exited 101. No production build or artifact copy follows this failed gate.

### Final clean-head gate and artifacts

After the boolean simplification and source rebase, clean HEAD was
`5455ebf90f8cd9d6e837d296820f6303a6a7cb66`. `just check-everything` passed,
including formatting, both clippy passes, the non-inheritable-socket check,
UI lint/typecheck, OpenAPI freshness, version/brand/naming/cross-drift checks,
and the 61-test BAAM registry plus 21-test privacy registry checks. The
previously bounded native suites remained green at 31/31 extension-install and
16/16 action-required route tests; the desktop suite was independently green at
551 files, 6,262 passed, and 19 skipped.

The final production build used `CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo
build --release -p biorouter-cli -p biorouter-server` and completed successfully
in 19m44s. Immutable artifacts are in
`/private/tmp/biorouter-crew-artifacts-5455ebf9/` with mode 0555:

- `biorouter`: Mach-O arm64, SHA-256
  `b329b6ad161e9b6722ef6b08ebef9aab3df4462c25c516a3206f051525fc7bdc`, version
  `1.91.1`.
- `biorouterd`: Mach-O arm64, SHA-256
  `e3bcbf581b4ec06f4a24e7a76a27f2843c36e9ef3e8b100087ab85021d5d5498`, version
  `biorouter-server 1.91.1`.

### Merged-main validation checkpoint

On merged HEAD `3dac36952ff4c822713bed3add9ba4f20005a64c`,
`just generate-openapi` completed successfully. The generated diff was limited
to the expected `SetWorkflowSlashCommandError` type export in
`ui/desktop/src/api/index.ts`; `openapi.json`, `sdk.gen.ts`, and `types.gen.ts`
were unchanged. Typecheck passed. The affected merged UI suite passed 6 files
and 60 tests, and the full desktop suite passed 562 files with 6,366 tests
passed and 19 skipped. The captured full-suite log is
`/tmp/desktop-vitest-merged-3dac3695.log`.

The merged `just check-everything` run passed through formatting, clippy,
socket inheritance, and UI lint/typecheck, then stopped at OpenAPI freshness
because the expected generated `index.ts` export was still uncommitted. No
merged native build was started.

### Observer backpressure regression checkpoint

After the merged observer transport patch and expiry sentinel fix, the focused
server module command
`cargo test -p biorouter-server --lib routes::crew_observation::tests` selected
12 tests and passed all 12, with 746 filtered and zero failures. This includes
regular-frame queue timeout fallback to the last accepted cursor, exact
terminal clear-error preservation, queued-data then one-terminal ordering,
closed-receiver no-fallback behavior, producer permit release, and expiry
cancellation classification. No native build was run for this checkpoint.

### Native continuation recovery focused tests (Luna)

The corrected child-process fixture was validated from the Crew worktree with
Hermit and the shared bounded target:

```text
source bin/activate-hermit && CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo test -p biorouter-cli --lib commands::shared_conversation::tests -- --nocapture --test-threads=1
```

Result: 11 selected tests passed, 0 failed, and 518 were filtered. The six
continuation cases run in fresh child processes under a parent-owned temporary
`BIOROUTER_PATH_ROOT`; each child binds its exact private daemon runtime socket
and creates its descriptor with `create_new`, so no existing runtime endpoint is
unlinked or replaced. The cases cover noninteractive refusal without mutation,
settling refusal, changed ownership, confirmed one-time cleanup, failed cleanup
with lease preservation, and uncertain admission consuming the lease without
automatic cleanup or retry.

The related daemon-client regression selection used:

```text
source bin/activate-hermit && CARGO_TARGET_DIR=/private/tmp/biorouter-crew-target CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo test -p biorouter-cli --lib daemon_client::tests -- --nocapture --test-threads=1
```

Result: 16 selected tests passed, 0 failed, and 513 were filtered. `git diff
--check` passed. The only daemon-client source change is the Unix test-only
`CrewClient::for_test` constructor; production descriptor, socket, peer-UID,
and identity checks remain unchanged.

Earlier attempts are excluded from evidence: an initial 11-test run was 9/11
because the temporary fixture had not published the required runtime descriptor;
a later 10/11 run used an unauthorized descriptor-discovery bypass and is
invalid. Neither result contributes to the counts above.

### Final native gate and debug artifacts (Luna)

On committed HEAD `532c3b7d954928b3ace92495d7a3e159efae6b9c`, the final native
`just check-everything` passed. It covered Rust formatting, both clippy passes,
non-inheritable socket checks, UI lint/typecheck/theme/contrast/token checks,
OpenAPI freshness, version/brand/Biorouter Copilot naming, vendored source and
cross-drift checks, and the 61-test registry plus 21-test privacy registry
checks.

The ordinary non-test debug build used
`CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 cargo build -p biorouter-cli -p
biorouter-server` and passed in 2m59s. Immutable 0555 artifacts are in
`/private/tmp/biorouter-crew-artifacts-532c3b7d/`:

- `biorouter`: Mach-O 64-bit arm64, SHA-256
  `edc9a59ba6afbc37df6b19d09070b3c931f4bd2a6de91f5f3c7965bd165efdef`,
  version `1.91.1`.
- `biorouterd`: Mach-O 64-bit arm64, SHA-256
  `4a1082ec9e1e0cc0b464c8cd608156a21cd24544c27cd87a2d42e7f84b16185c`,
  version `biorouter-server 1.91.1`.

### Native PTY continuation acceptance on immutable debug pair (Luna)

Using the immutable debug pair in `/private/tmp/biorouter-crew-artifacts-532c3b7d/`, a fresh synthetic profile was launched with a shared daemon, private runtime descriptor, API secret, and a 39-character approval secret supplied through the daemon's stdin digest path. A loopback-only OpenAI-compatible HTTP fixture supplied deterministic streaming responses; no real provider, credential, or private fixture was used.

The normal daemon APIs created active turns and admitted Stop-and-Send cancellation with `expected_turn_id`, `wait_for_idle`, `continuation_pending`, and a proven `X-User-Action` header. The CLI acceptance covered:

- noninteractive shared resume with text: refused the pending continuation before dispatch, with `resubmit_automatically:false`;
- interactive `leave`: refused with `Pending continuation left unchanged; input was not submitted`;
- interactive `abandon`: resolved the claim and returned to an idle session;
- interactive `takeover`: claimed the exact retired generation, accepted one successor input, and persisted the deterministic `successor` assistant response;
- post-success resume: reported no active turn and no pending continuation.

The flow used 1 noninteractive refusal, 1 leave refusal, 1 abandonment, 1 takeover, and 1 successful successor turn. The final transcript had no pending continuation and the successor was visible in the authoritative session response.

### Hosted clean-install failure and lockfile repair

At published `148415990b6126960517cab64ae3f71ae9e294b1`, seven hosted
frontend/serve checks stopped before their test assertions because npm 11.19.0
rejected the desktop lockfile: `encoding@0.1.13` and its nested
`iconv-lite@0.6.3` were missing. The downstream missing TypeScript, lint tooling
and preview evidence errors did not establish independent product defects.

Astra regenerated the lock in an isolated manifest directory with
`npm exec --yes --package=npm@11.19.0 -- npm install --package-lock-only
--ignore-scripts --no-audit --no-fund`. Independent Astra review confirmed only
the two optional development packages were added; 35 existing entries changed
only peer metadata. Existing package versions, resolved URLs, integrity hashes
and dependency edges are unchanged. Hermit npm 11.6.1 regeneration had left the
original incomplete lock unchanged.

Luna then passed separate clean `npm ci --ignore-scripts --no-audit --no-fund`
installs with npm 11.19.0 and 11.6.1, both under local Node 26.9.0. Hermit
activation in that agent was blocked by cache metadata permissions, so these
are not claimed as Node 24 runtime tests. UI typecheck and lint passed, including
ESLint, theme checks, 404 contrast checks and token mirrors; diff whitespace
checks passed. Hosted Node 24 confirmation remains pending on the repaired
revision. No full UI suite was repeated for this lock-only repair.

### Windows authentication fixture compilation repair

Hosted Windows MSVC Rust 1.92.0 on `14841599` stopped before assertions:
`crew/authentication.rs`'s test-only `FakeChild` omitted the Windows-required
`portable_pty::Child::as_raw_handle` method. Luna added the Windows-only method
returning `None`, representing a synthetic child without an OS handle. Astra
reviewed the five-line test-only change; production authentication and existing
assertions are unchanged. Formatting checks passed.

A follow-up focused authentication test attempt in that agent could not resolve
crates.io's `chacha20poly1305`, and offline mode lacked `cap-std`; no passing
count is claimed for that attempt. It used two Cargo jobs and disabled
incremental compilation. The Windows-specific method requires confirmation by
the next native Windows hosted build; a Unix test cannot establish that target's
compilation or execution.

### macOS arm64 debug app package (Luna)

Built locally without launching the app or modifying installed Applications:

```text
source bin/activate-hermit && CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0 just copy-binary debug
source bin/activate-hermit && GOMAXPROCS=2 python3 scripts/computer-use-runtime.py build darwin-arm64
source bin/activate-hermit && cd ui/desktop && GOMAXPROCS=2 node scripts/prepare-platform-binaries.js
source bin/activate-hermit && cd ui/desktop && GOMAXPROCS=2 npm run package
```

All four commands completed successfully. The reviewable app is
`ui/desktop/out/Biorouter-darwin-arm64/Biorouter.app` (1.0G). The embedded
`Resources/bin/biorouter` and `biorouterd` are Mach-O arm64 and match the
signed debug binaries copied from `target/debug`:

- `biorouter`: SHA-256 `d5cf729d5e5db26cd604cb59ef02ac8c7a50bbee3e09c29413998c4d9a6c02a8`
- `biorouterd`: SHA-256 `d79ade3fd2d2f757651ff6e6970c6cd416433eecd15865142c381a66f5468ef7`

The pre-package immutable 532c pair remains preserved at
`/private/tmp/biorouter-crew-artifacts-532c3b7d/`; `just copy-binary debug`
signed the working debug binaries before packaging, so the embedded hashes
match the signed working targets rather than the pre-sign immutable hashes.
The pinned Computer Use runtime manifest is present at
`Contents/Resources/computer-use/manifest.json` (SHA-256
`827bc16024c1ad840a6ebddc10b566bd90637fcd6cb3d6b879f1b756323248f9f`).

Electron Forge packaging completed for arm64. `codesign --verify --deep
--strict` currently fails with `invalid Info.plist (plist or signature have
been modified)`, so the packaged app is unsigned/invalid for distribution;
no signing identity or notarization was applied by `npm run package`.

### Disposable signed development app fixtures (Luna)

The packaged app was copied without changing the original into
`/private/tmp/biorouter-crew-dev-apps-c857cc1b-1/`:

- `BioRouter-Crew-QA-Alice.app` — bundle id `dev.biorouter.crew.qa.alice`, display/name `BioRouter Crew QA Alice`.
- `BioRouter-Crew-QA-Bob.app` — bundle id `dev.biorouter.crew.qa.bob`, display/name `BioRouter Crew QA Bob`.
- `BioRouter-Crew-QA-Carol.app` — bundle id `dev.biorouter.crew.qa.carol`, display/name `BioRouter Crew QA Carol`.

Each copy was ad-hoc signed recursively with `codesign --force --deep
--sign - --timestamp=none` and passed
`codesign --verify --deep --strict`: valid on disk and satisfies its
Designated Requirement. `TeamIdentifier` is unset, as expected for ad-hoc
signing; no distribution certificate, notarization, LS registration, or user
Application copy was used.

The embedded binaries are identical across all three copies and match the
signed package source: `biorouter` SHA-256
`d5cf729d5e5db26cd604cb59ef02ac8c7a50bbee3e09c29413998c4d9a6c02a8`, and
`biorouterd` SHA-256
`d79ade3fd2d2f757651ff6e6970c6cd416433eecd15865142c381a66f5468ef7`. These
hashes differ from the pre-sign immutable 532c pair only because
`just copy-binary debug` applied the stable local signing identity before
packaging; the code payload is unchanged. The original packaged app and
immutable pair remain preserved.

### Empirical 532c payload provenance check

To distinguish signing metadata from code payload, I copied the immutable 532c
CLI/daemon and the packaged signed CLI/daemon into
`/private/tmp/biorouter-crew-dev-apps-c857cc1b-compare/` and removed signatures
from those copies only. Whole-file hashes still differed because Mach-O load
commands and `__LINKEDIT` signature metadata differ. I then parsed each copy's
Mach-O section table with `otool -l`, concatenated every initialized non-
`__LINKEDIT` section (excluding zero-fill `__thread_bss`, `__common`, and
`__bss`), and compared both bytes and SHA-256:

- CLI: 216,006,659 initialized payload bytes; both copies
  `068bf8b2407a957dcddd655e8a621cbd6517323f4143a15318d356d5c5398ce8`;
  byte-for-byte identical.
- Daemon: 222,773,451 initialized payload bytes; both copies
  `52514af3bde67862eed1f11f336be155a3045643b9c9d5db9e0e7c0095b037df`;
  byte-for-byte identical.

Thus the initialized code/data sections match the immutable 532c pair. This
comparison excludes load commands and `__LINKEDIT`; it is not a whole-file
identity check or packaged runtime acceptance. The three prepared fixtures
remain at `/private/tmp/biorouter-crew-dev-apps-c857cc1b-1/` with bundle IDs
`dev.biorouter.crew.qa.alice`, `.bob`, and `.carol`; each passed strict ad-hoc
signature verification.

### daemonRuntime fixture portability refresh (Luna)

The focused regression fixture now creates its synthetic root below
`os.tmpdir()` via `path.join(os.tmpdir(), 'br-runtime-')`, rather than assuming
the macOS-only `/private/tmp` path. Its profile stores the canonicalized
configuration directory from `fs.realpathSync(config)` so macOS `/var`
symlink normalization matches the daemon identity check.

Bounded validation passed:

```text
npm --prefix ui/desktop exec -- vitest run src/daemonRuntime.regression.test.ts --maxWorkers=2 --reporter=verbose
1 file passed; 7 tests passed

npm --prefix ui/desktop run typecheck
passed

npm --prefix ui/desktop run lint:check
passed (typecheck, ESLint, themes, contrast, and token checks)
```

The focused Vitest command required elevated execution because the local
sandbox denied Unix-domain socket binding; no app process or production source
was changed.

### PR #366 hosted Rust test triage (run 35812337881)

The finalized workspace library and binary test jobs exposed deterministic
source/fixture guard failures after the earlier Windows `FakeChild` portability
compile fix.

Ubuntu job `107026656579` reported `4243 passed; 2 failed; 2 ignored`:

- `privacy::system_auth::tests::no_caller_raises_a_prompt_without_a_bound_on_it`
  at `crates/biorouter/src/privacy/system_auth.rs:774`; the lexical scan matched
  `.authenticate(` in `crates/biorouter-cli/src/commands/crew/mod.rs`. Source
  review identified a call to Crew's SSH terminal controller, not the OS
  authentication prompter. Renaming it `authenticate_ssh` disambiguates the
  operation without changing its body, proof checks or the audit.
- `utils::tests::the_untrusted_label_sanitizer_is_defined_exactly_once` at
  `crates/biorouter/src/utils.rs:294`; the drop set was also present in
  `crates/biorouter-cli/src/commands/crew/output.rs` and
  `crates/biorouter-cli/src/commands/shared_conversation.rs`.

Windows job `107026656565` reported `4133 passed; 5 failed; 1 ignored`.
It repeated both failures above and added three `crew::ssh_policy` failures:
`native_preflight_accepts_safe_two_hop_config` (line 427),
`native_preflight_rejects_weak_implicit_jump_before_connecting` (line 459),
and `native_preflight_rejects_jump_cycle` (line 474). Each rejected the
Windows profile path as shell-sensitive before the test's expected assertion.
The test-only `FakeChild::as_raw_handle` change compiled successfully; these
failures occurred later during the workspace test run.

macOS job `107026656510` independently reproduced the same two guard failures:
`4253 passed; 2 failed; 2 ignored`. No additional macOS-specific failure was
reported.

### Final focused regression lane after hosted-failure fixes (Luna)

Using the established Hermit environment with `CARGO_BUILD_JOBS=2
CARGO_INCREMENTAL=0`, the bounded focused suites passed:

- `cargo test -p biorouter --lib crew::ssh_policy -- --nocapture`: 10 passed, 0 failed, 4,241 filtered.
- `cargo test -p biorouter --lib crew::authentication -- --nocapture`: 6 passed, 0 failed, 4,245 filtered.
- `cargo test -p biorouter-cli --lib commands::crew::output -- --nocapture`: 5 passed, 0 failed, 527 filtered.
- `cargo test -p biorouter-cli --lib commands::shared_conversation::tests -- --nocapture --test-threads=1`: 12 passed, 0 failed, 520 filtered.
- `cargo test -p biorouter-cli --lib daemon_client::tests -- --nocapture --test-threads=1`: 16 passed, 0 failed, 516 filtered.

The shared-conversation test module was missing imports for its own private
`json_terminal_safe` and `terminal_control` helpers; adding those test-module
imports fixed the only compile error. No production behavior changed.

The final `source bin/activate-hermit && CARGO_BUILD_JOBS=2
CARGO_INCREMENTAL=0 just check-everything` passed all formatting, clippy,
non-inheritable-socket, UI lint/typecheck/theme/contrast/token, OpenAPI
freshness, version/brand/naming, vendored-source, cross-drift, registry, and
privacy-registry checks.

### Exact source-audit tests (Luna)

The two requested exact audits passed under Hermit with
`CARGO_BUILD_JOBS=2 CARGO_INCREMENTAL=0`:

- `cargo test -p biorouter --lib privacy::system_auth::tests::no_caller_raises_a_prompt_without_a_bound_on_it -- --exact --nocapture`: 1 passed, 0 failed, 4,250 filtered.
- `cargo test -p biorouter --lib utils::tests::the_untrusted_label_sanitizer_is_defined_exactly_once -- --exact --nocapture`: 1 passed, 0 failed, 4,250 filtered.
