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

### Published 7ab40c81 native and hosted verification

The ordinary non-test native build and focused SSH/Unicode runtime smoke
passed; [artifact hashes and exact scope](evidence/cli-7ab40c81-20260923.md)
include JSON round-trip and terminal safety checks for U+E0041. The daemon is
byte-identical to the prior 532c3b7d artifact; this does not replace the remaining
graphical, AWS or cross-platform acceptance gates.

Hosted Rust run `35815167642` passed macOS. Windows's core library target
reported 4,136 passed, three failed and one ignored: the three native SSH
fixtures still supplied a configuration path outside the admitted grammar.
The actual rejected path was not captured, so a verbatim-prefix explanation is
unproven. The follow-up test construction prefers the runner's temporary root,
preserves unsafe-path rejection and includes an escaped diagnostic for the
fixture path. No production SSH path grammar is widened.

Ubuntu progressed to its integration stage, which found three fixture issues:
the FIFO test assumed `/private/tmp`, the separately included Unix transport
test helper lacked child-command preparation visible to the source census,
and the Crew transfer integration binary omitted the required test sandbox.
The fixes use the portable temporary directory, prepare that synthetic child,
and declare the existing sandbox. The source censuses and production behavior
are unchanged. Hosted Vitest and the frontend/static/API/browser/Electron
checks passed on 7ab40c81. Frontend run `35815167669`, Vitest job
`107035042911`, reports 563 files passed, 6,370 tests passed and 19 skipped
(6,389 total). All seven Frontend jobs succeeded. The subsequent fixture
corrections require their own focused and hosted verification.

### Focused test-only fixture portability lane (Luna)

After the 7ab40c81 source integration, the four released integration fixtures and
core regression filters passed under Hermit with `CARGO_BUILD_JOBS=2`
and `CARGO_INCREMENTAL=0`:

- `cargo test -p biorouter --test daemon_runtime_contract -- --nocapture`: 7 passed, 0 failed, 0 ignored.
- `cargo test -p biorouter-mcp --test no_console_window_census -- --nocapture`: 20 passed, 0 failed, 0 ignored.
- `cargo test -p biorouter-server --test every_test_binary_is_sandboxed -- --nocapture`: 5 passed, 0 failed, 0 ignored.
- `cargo test -p biorouter-server --test crew_transfer_authority -- --nocapture`: 10 passed, 0 failed, 0 ignored.
- `cargo test -p biorouter --lib crew::transport::tests -- --nocapture`: 12 passed, 0 failed, 4,239 filtered.
- `cargo test -p biorouter --lib crew::ssh_policy -- --nocapture`: 10 passed, 0 failed, 4,241 filtered.

The six commands selected 64 tests in total, all passing. These are test-only
fixture portability and source-guard changes; production behavior was not edited.
Windows-native confirmation remains a separate hosted requirement.

The post-fix `source bin/activate-hermit && CARGO_BUILD_JOBS=2
CARGO_INCREMENTAL=0 just check-everything` gate also passed. It completed Rust
formatting and clippy, non-inheritable socket checks, UI lint/typecheck/theme/
contrast/token checks, OpenAPI freshness, version/brand/naming/vendored-source/
cross-drift checks, and registry/privacy-registry checks (61/61 and 21/21 Node
assertions respectively). No generated API diff remained.

### Shared-workflow source closure on dd70051e

An independent GPT-6 Astra source review found no actionable finding in the
audited connection/authentication, transfer, task and context parity seams.
The daemon owns authentication and verified master adoption; successful GUI
completion releases the terminal adapter without cancelling the adopted master
(`crew/authentication.rs:406`, desktop `main.ts:5056`). Both interfaces use the
same Proven-only transfer endpoints; the daemon owns target approval, durable
receipts, execution and recovery (`routes/crew_transfers.rs:27`,
`crew/transfers.rs:649`). Task launch/cancellation and grant/provider/source
policy remain shared, and explicit `run/session --shared-daemon` reaches daemon
conversation APIs before local agent/tool-bridge creation (`cli.rs:2237`,
`routes/crew.rs:1216`, core `crew/mod.rs:1337`).

This closes the bounded source review, not runtime acceptance. Mixed GUI/CLI,
native Save, broader privacy/recovery races and final artifact qualification
remain open. The reviewer performed no tests, builds or edits.

A subsequent bounded Luna run on the recorded `7ab40c81` pair used supported
`Initial::All` observation with no cursor, positively verified the existing
A+B-derived message, and revoked only the source membership. The stream emitted
`policy_changed` with `clear:true`, followed by CLI exit 1; neither of the two
new owner controls reached that observer. The derived/history bytes had already
reached the CLI pipe while authorized, so the full daemon-queued race remains
unqualified. [Exact scope and frame](evidence/cli-7ab40c81-20260923.md#initial-history-and-source-revocation-terminal-behavior)
are recorded separately from the earlier after-derived-cursor attempt.

### Hosted closure and prepared GUI refresh

Published `dd70051e90d4935fcc908cc28dbdb144f9e79ce5` passes Rust run
`35818114269` on Windows, Ubuntu and macOS, including the previously failing
Windows SSH fixtures and Ubuntu FIFO/child-census/sandbox-census assertions.
Both cross-checks, serving, guards, native payload jobs and the commit-message
check pass. Frontend run `35818114160` passes all seven jobs, with 563 Vitest
files, 6,370 tests passed and 19 skipped. The optional nightly cross-build is
skipped. [Per-target counts and run links](evidence/ci-dd70051e-20260923.md)
avoid summing overlapping platform and library/binary targets.

The three prepared QA apps now embed the exact `7ab40c81` native pair and pass
ad-hoc deep/strict signature verification. [Preparation evidence](evidence/prepared-gui-7ab40c81-20260923.md)
records hashes and source equivalence for unchanged production desktop code.
No app was launched or registered, and no GUI/AWS acceptance is inferred.

The ordinary Linux ARM64 CLI/daemon pair was also rebuilt from an immutable
`dd70051e` archive with the pinned Rust image, locked dependencies, two jobs
and incremental compilation disabled (4m57s). A fresh UID 1101 tmpfs profile
passed start/status, wrong-valid-proof HTTP 403, stop, restart with the same
profile and a new instance, final stop and missing-daemon status. The daemon
hash matches the recorded 532 Linux daemon; the CLI includes the later Unicode
output correction. [Linux provenance and hashes](evidence/linux-dd70051e-20260923.md)
retain the GLIBC 2.39 limit and distinguish this local container from AWS or
institutional acceptance.

## Approved final acceptance execution — current checkpoint ae103eb5

The user has explicitly approved the previously blocked native QA app control and transfer of verified binaries plus synthetic data to disposable AWS. The earlier automatic-review rejections and cleanup records above remain accurate for their historical checkpoints; this authorization does not turn those attempts into passes or authorize broader source/visual export. All required hosted CI checks pass on `ae103eb5`; PR #366 remains draft.

The `ae103eb5` working tree now includes an uncommitted observer correction, independently reviewed with no findings. Local provenance is `/private/tmp/crew-observer-artifacts-ibQ0ex/PROVENANCE.md`; observer source SHA-256 is `041ace5802ed32ea4195600488ea39a2b219301e83d780f7307f4d4a7b24524a`. Commands recorded there passed: focused `routes::crew_observation::tests` (15 passed, 746 filtered), strict server Clippy, native CLI/server debug build and `just check-everything`. API regeneration produced no tracked schema diff.

| Native artifact | SHA-256 |
| --- | --- |
| CLI | `278db11f673b13ec866d9209a349610e1deda8f7800dc3eec36958b9b615ad3f` |
| Daemon | `829efc9765a62b89d3eebe0f86baa6d3c8514a58d1f03fe6409869b141992227` |

These tests exercise real receiver admission with deterministic queues: missing-Proven rejection, blocked canary suppression, injected terminal clear-error priority and permit release. They do not establish live broker source-ACL revocation. The real-source-ACL module now compiles; the full observer filter reports 15 passed and one ignored. Its actual live execution remains pending.

The AWS receipt at `/private/tmp/biorouter-crew-luna-20260923T055757Z-9298d2b5/luna-acceptance-report.md` records three ordinary users (UIDs 10001/10002/10003), distinct keys, verified user-local binary installation/help, rootless broker start/stop/restart with stable workspace identity, and cross-owner descriptor/journal/state refusal. This is three users, not three UIs. Initial invalid bootstrap-key attempts remain failed setup attempts; no teams/messages or end-to-end collaboration pass is inferred. The instance is still live; persistent cleanup supervisor 72714 replaces 61914 and has an absolute 2026-09-23 07:45:00 UTC deadline, and teardown verification remains pending.

Packaged QA launch was refused by the intended installed-build development-profile guard. The supported stock-Electron development shell was selected; actual GUI readiness has not passed. Record subsequent actual outcomes and cleanup below without upgrading these setup results.

| Final acceptance area | Current result | Remaining evidence |
| --- | --- | --- |
| Actual queued source-ACL revocation | Module compiled; live test ignored, execution pending | Real broker/ACL mutation and queued delivery assertions |
| Native app control, Save and mixed GUI/CLI | Approved; supported development-shell readiness pending | Exact app/backend identity and actual three-user UI outcomes |
| Disposable AWS product workflow | Three-user install/rootless lifecycle/state denials pass | Actual collaboration/privacy/transfer workflow and independently verified cleanup |

### Dirty-snapshot Linux refresh and GUI readiness follow-up

The report `/private/tmp/biorouter-crew-linux-ae103eb5/report.md` records an ordinary non-test ARM64 build from the `ae103eb5` base plus uncommitted observer snapshot. Source archive SHA-256 is `5f80aae7b0668067967dbaa7da04f630dd148ac3da4ddd317b5461417eff71bf`; patch SHA-256 is `159437741db88e976e5a8592ffd7dd1c66e4cd4c3457c0f25310bf1a08ffb75c`. The build used source-pinned Rust 1.92.0 rather than image-default 1.98.1, locked dependencies and two jobs, and passed in 7m53s. Mode-0555 ARM64 ELF copies import at most GLIBC 2.39; version/help checks pass. This lane performed no lifecycle or AWS acceptance test.

| Refreshed Linux artifact | SHA-256 |
| --- | --- |
| CLI | `2e9086669a5274b5a953c184b23660ab07403d0c01c9ad12e87fba5c697db567` |
| Daemon | `fe9cbf9db6adeb1cf396930ddf2f7c70846d3b9ebaa8972ebc8a76e02a650809` |

Exact refreshed-pair AWS transfer and UID 10001 lifecycle pass with matching hashes; this is headless lifecycle evidence, not collaboration acceptance. The AWS instance remains live under the 07:45:00 UTC cleanup deadline. GUI follow-up observed a 180-second first-run osascript prompt timeout and detached child reaping; a supported persistent foreground launch with explicit isolated HOME/userdata is pending. No completed GUI readiness is claimed.

### Refreshed-pair AWS lifecycle and supervisor confirmation

The refreshed AWS receipt records matching reviewed Linux CLI/daemon hashes on UID 10001. A fresh profile completed start/status, wrong valid-format proof refusal (403), correct proof stop (200), missing-daemon status, restart with stable profile `09d75327-870f-4dbd-ae79-217ad2233d1c` and new instance `5875bd4a-37b6-46e5-8f28-10c740e2e64b`, final stop and missing-daemon status. The temporary approval file was removed; the superseded owned daemon was stopped by exact PID. This establishes bounded updated-pair headless lifecycle only, not GUI or queued source-ACL acceptance.

Post-live-module strict Clippy including tests, formatting and the socket census across all 17 production targets pass. Actual queued source-ACL execution remains pending. AWS owner confirmed persistent supervisor session 72714 replaced 61914 and targets absolute 2026-09-23 07:45:00 UTC. The instance is still live; current cleanup is pending, distinct from the earlier 13:40 UTC historical cleanup pass.

### Pushed observer checkpoint and native launch follow-up

Observer checkpoint `702e9e7a` is pushed to draft PR #366 with CI pending; preceding `ae103eb5` required CI passed. Native macOS source fixes are uncommitted: current-host osascript activation, explicit 180-second timeout errors and asynchronous fatal startup dialog/error logging. Five additive native-prompt tests and `just check-everything` pass.

The home13 sample and owned-process cleanup confirmed an NSAlert modal after approval cancellation with no daemon. The modal fatal-dialog path delayed the error log; this cause is established for that launch symptom. The first refreshed `build-main.js` output lacked `MAIN_WINDOW_VITE_DEV_SERVER_URL`; home14 launch was invalid and started no daemon. The supported `npm run build:e2e` then completed, producing main SHA-256 `38b9af82d15ffa589da82463ac9cb0000526c52bc988c511fbe187fc6f49ecbd` and preload `333e4cdf8360751de05e46b0812e09594f0bc2fee6f7d782a623ada8d92334d3`. Home15 GUI retry remains pending; no qualifying UI acceptance is recorded.

The actual queued source-ACL test remains unrun because native SSH reports `ssh_eof` while the exact direct SSH hello succeeds. Fresh escalated-daemon reproduction excludes the prior restriction explanation, but does not establish a product SSH root cause. Diagnosis is active. The AWS instance remains live under absolute 2026-09-23 07:45:00 UTC cleanup supervisor 72714; teardown is not yet verified.

### Source checkpoint 9111c3e9: corrected fatal logging and SSH diagnosis

The earlier asynchronous-dialog remedy is superseded by examination of exact Electron 39.8.10: parentless `showMessageBox` still invokes `NSAlert.runModal`. The source fix in this checkpoint restores the original dialog and performs a guarded fatal-only synchronous log append before modal presentation; ordinary logs remain asynchronous. Logger/prompt focused checks pass 8/8 and the full local gate passes. Full Forge build main SHA-256 is `94a0f12cdb9d0fe332098da32ec8a7e29140d14f18ee7f4105edd1c6501a9cdb`. These results belong to source checkpoint `9111c3e9` plus those fatal-log source changes prepared for review/commit, not a future documentation commit.

CUA rejected `/usr/bin/osascript` as “Invalid app”. The successful profile is native-home19, not home18. Home18 logged an invalid-secret error at 07:33. The user entered both dialogs for home19; root verified Electron PID 51007, renderer PID 51246 and daemon PID 51244 running from 07:35:25, API port 61630, matching process arguments and a clean home19/electron/logs/main.log startup. No additional approval is needed. This verifies startup only: native CUA and actual GUI-drive acceptance remain unqualified. Root interrupted the long silent GUI-agent call to obtain status; no completed main-window interaction or workflow is inferred.

Fresh native SSH logs showed the current short path. The saved socket had a 33-hex typo while the actual socket path used 32 hex characters; the supported connection update restored Alice’s connection. This rules out the proposed native SSH source bug for this failure. Prior 64-hex/control-master conclusions are excluded. Actual queued source-ACL execution remains pending on clean fresh profiles. AWS cleanup deadline is now absolute 2026-09-23 08:15:00 UTC under supervisor 7908; current teardown remains pending.


### Live queued revocation and UI-driver checkpoint, 2026-09-23

The [fresh real-broker acceptance](evidence/queued-source-acl-20260923.md) supersedes the preceding unrun status for queued source revocation. One selected ignored test passed in 1.09 seconds, with 761 filtered. The ordinary observer filter passed 15 tests with 747 filtered. The privacy census failure on `9111c3e9` was a test-file classification defect; renaming the harness to `crew_observation_live_acceptance_tests.rs` and updating its test-only module path makes the focused `the_scan_really_separates_production_from_tests` check pass. The production privacy scanner was not relaxed. Rust formatting and compile-only validation passed after the rename. The evidence records accepted error alternatives and the conditional epoch assertion, not unprinted numeric values or a selected error code. Development plaintext credentials and test-only human proof limit this to real SSH/broker/observer behavior, excluding encrypted-vault and HTTP-authentication qualification.

Native fatal logging is published in `dc09f28f`, with eight focused tests and independent review. The user completed native-home19 bootstrap. Native CUA bundle-ID/path selection hung and name selection was refused. The supported Electron Playwright driver works after a controlled CDP-enabled restart and has driven the actual Crew sidebar, workspace preparation and private-connection form. This is actual UI evidence, but three-user collaboration and native Save remain open. Alice's UI-created AWS workspace is `2b580016-0f01-4afc-93af-95645ee5118c`. Supplying the isolated synthetic SSH config fixed Connect. The current descriptor and process record retain daemon PID 51244 across the Electron restart; the changed local API port belongs to the new proxy. Normal authenticated workspace snapshot reads succeed, while an eight-second observer probe returned no headers or bytes before cancellation; compression, proxy and stream latency are being compared before attributing a product defect.

The same disposable AWS resources remain available under persistent supervisor 84843 until absolute 2026-09-23 09:00:00 UTC. It replaces supervisor 7908 without adding resources. Cleanup remains unverified until the instance, volume, security group, keypair and local private-key teardown checks complete.

The final pre-push `CARGO_BUILD_JOBS=2 just check-everything` gate passed with exit 0 after the live-test rename/connect changes. The worktree target was used; no additional broad local workspace test suite was launched. The scoped evidence report records one live test, fifteen observer tests and the passing focused privacy census.

### Actual Alice UI connection and NDJSON compression investigation

The report `/private/tmp/crew-alice-playwright-observer-report.md` records actual renderer driving, distinct from its subsequent diagnostic fetches. Alice native-home19 uses Electron PID 76672, daemon PID 51244, local proxy API 63070 and connection `0cfdfb9e-7011-457c-9333-94ec0ebac9cb`. UI Connect succeeds using isolated synthetic SSH configuration. Normal authenticated `workspace.snapshot` returns 200; its positive control completed in 73 ms. The UI-created workspace remains `2b580016-0f01-4afc-93af-95645ee5118c`, with one principal and no teams/channels at this observation.

The app-generated observer request uses `{"initial":"latest"}` and returns HTTP 200, `application/x-ndjson`, gzip, chunked and no-store. No state frame arrives, so the UI remains on the first-connection screen. Separate renderer-authenticated probes requested identity/gzip encodings but timed out near eight seconds without headers/reader/bytes; a 30-second probe likewise aborted at 30,033 ms with zero bytes. These are diagnostics, not successful workflow steps. Browsers forbid caller control of Accept-Encoding; the requested identity/gzip values were ineffective and do not establish an identity-encoded renderer response.

`/private/tmp/crew-live-luna/gzip_probe_incremental_receipt.json` records HTTP 200 gzip NDJSON with ten initial raw bytes at 2,551.14 ms and no decoded frame type/code or first-decoded timestamp. The proper incremental gzip probe produced no decoded frame through ten seconds. In contrast, a direct Unix-socket identity-encoding control returned HTTP 200 and decoded the first 623-byte state in approximately 2.5 seconds. This comparison confirms the compression-buffering defect; the renderer probes alone were not identity controls and no GUI initial-state delivery is established. The current `commands/agent.rs` source change excludes NDJSON through the real `response_compression` helper. Luna testing/build is active; no correction pass is recorded yet.

Published `7e2ff404` queued-source ACL acceptance remains valid within its recorded scope. Next steps are focused compression verification, matching artifact rebuild and actual renderer first-state replay before further three-user mixed UI/CLI or Save acceptance. No new GUI workflow, authority or AWS cleanup result is inferred from the probes.

### Compression correction: verified helper and build checkpoint

Three tests of the actual production `response_compression` helper pass: large valid JSON remains gzip-compressed, an open NDJSON response delivers its first frame uncompressed, and an open SSE response delivers its first frame uncompressed. The ordinary daemon build and full `just check-everything` gate pass; no dependency diff was introduced. Focused review found no actionable ownership/privacy findings and authentication behavior is unchanged.

The immutable pair is `/private/tmp/crew-gui-pair-20260923T0842Z`: daemon SHA-256 `0890a909cd1e6933ae6aa06549275e5d0152a72c006b21a7bb713de0c7ee2cf6`; unchanged CLI SHA-256 `278db11f673b13ec866d9209a349610e1deda8f7800dc3eec36958b9b615ad3f`. This is build/focused-helper evidence, not post-fix GUI acceptance. The GUI lane authenticated the stop of old daemon PID 51244 (instance `57b9b23b-465a-49f0-907b-0b003b734a6e`). New immutable daemon PID 71261 has instance `dd102a5d-e5e4-4877-9faa-e8c56f3a9c53`, the recorded `0890a909…` hash and `user_action_installed:true`; profile `8f70f965-a74a-4f22-b9d4-0b7662d625fd` and runtime/socket are preserved. Alice Electron is deliberately closed pending manual readiness and one attach prompt. Actual renderer replay remains pending; no post-fix GUI pass is established. The separate owned ACL fixture now passes a live HTTP retest: HTTP 200, no Content-Encoding, and an initial state in 2,390.2 ms despite requesting gzip. Its positive snapshot returns HTTP 200 with 2,230 bytes. Schema generation passes with no tracked API diff; [the scoped evidence](evidence/ndjson-compression-20260923.md) records exact commands, counts, hashes and fixture boundaries.

AWS cleanup supervisor 40377 replaces 84843 and targets absolute 2026-09-23 09:45:00 UTC. Mandatory teardown remains unverified; earlier deadlines and supervisor records above are historical.

### Verified Linux package checkpoint 6c5a8640 and new GUI fixture

At 2026-09-23 16:19 UTC, root verified every required PR check successful on exact `6c5a8640`: macOS Rust 24m35s, Linux Rust 32m32s and Windows Rust 20m49s, plus Vitest/static/OpenAPI/cross/native checks. Cross-build-nightly was intentionally skipped. This is hosted validation, not GUI acceptance. Exact-head Linux package workflow 35840101914 succeeded on `6c5a8640fedf896277c3067d6c992e3e323e9a78`. Backend job 107112864999 finished at 09:27:12 UTC; package job 107122378956 finished at 09:36:09 UTC, including GLIBC-floor and packaged-payload checks. Ordinary backend archive artifact ID 10741942349 is 121,709,522 bytes, SHA-256 `fabf2115f5baa9ca19c2446172427fb1d828184312403b0b70a21361d5620a5b`. The verified manifest is `/private/tmp/crew-linux-6c5a8640-ci/verified/VERIFIED-MANIFEST.json`.

| Ordinary artifact | SHA-256 | Qualification |
| --- | --- | --- |
| CLI | `fe9a1f34863bfe51c35e23eae41644108fa22cb4734c964e2d47b0b545087ec9` | ELF64 x86-64 PIE, mode 0555, max GLIBC 2.30 |
| Daemon | `f72b4753405d18af5affe27982e7a283036c75151961b4b627045d3364ea17a1` | ELF64 x86-64 PIE, mode 0555, max GLIBC 2.30 |
| Crew broker | `edc3133a8d80489b26497374d32472690179247b33394f0423ef7955fd81aac4` | ELF64 x86-64 PIE, mode 0555, max GLIBC 2.30 |

These binaries were not executed on the macOS verification host. Prior AWS cleanup was verified true at 09:45:27 UTC, with fresh absence checks. New fixture provenance is `/private/tmp/crew-three-user-20260923T1606Z-luna/fixture-state.json`: Ubuntu 24.04 x86-64 t3.small, UIDs 10001/10002/10003, strict known-host checking, caller-/32 SSH ingress, encrypted delete-on-termination EBS and IMDSv2. Cleanup supervisor 17881 targets 18:30 UTC; new-fixture cleanup remains pending.

Actual Alice Welcome → Crew navigation is verified; the old connection is disconnected as expected. The earlier keychain failure was a test-setup mismatch in a development-credential profile, not an established product bug or encrypted-vault test. Authenticated stop of old instance `dd102a5d…` returned rc 0. The subsequent PID 77698 was manually launched with `biorouterd agent`, without shared runtime; it was not supported `crew daemon start` and does not establish preserved shared-profile or lifecycle behavior. Root requested cleanup of that exact stray process. After native user input, Electron started actual daemon PID 79280. Playwright Prepare Hosting Identity returned HTTP 200 at 2026-09-23 16:22:38 UTC and its valid public key was supplied to AWS. This is actual UI preparation evidence only; CLI status still needs to prove the app daemon’s profile/UDS identity before same-instance attachment or shared-lifecycle acceptance. No registry credential edits, product defect or three-user GUI pass are claimed.

### Actual post-fix Alice observer state and supported AWS CLI lifecycle

Actual Alice post-fix UI state rendering passes: after installing the helper at the supported `~/.local/bin` path, the new connection returned Connect 200, explicit Initialize as workspace host succeeded, and the UI created “Crew Acceptance Team” and rendered private policy, `#general`, one member, SSH identity `crew_alice` and the message composer. This is authorized observer-state rendering, not a substituted snapshot-API result: `CrewView.tsx` sets a non-null snapshot from the `observeCrew` state frame; refresh reloads connections and restarts observation. The generic stale-observer alert before bootstrap is a minor UX finding while no authorized snapshot exists; no fix for that alert is claimed. Alice’s supported CLI status and human-proof connections list match app-owned daemon PID 79280, profile `8f70f965-a74a-4f22-b9d4-0b7662d625fd` and instance `53472f2e-61f2-4c82-865a-32ac8470d1e6`. CLI post `2ca68065-f728-4316-8cfd-f988ea9ef04d` matches exact history; the actual Alice GUI rendered that message. Alice then sent `GUI parity reply 20260923T1703Z` through the UI; native CLI history confirmed exact message `556ff02b-f330-4507-bb6a-960371f81c04` and body. This is bounded bidirectional Alice GUI/CLI parity, not three-user collaboration. This closes identity/discovery only, not detach/reopen lifecycle. Bob PID 6443/CDP 9223/native-home20 and Carol PID 9422/CDP 9224/native-home21 accepted manual native attachment and render Crew; all three apps are live, without a three-user collaboration pass. Three-GUI collaboration, native Save, agent/privacy and broader fault gates remain open; secrets and screenshots remain local-only.

On exact `6c5a8640` Linux binaries, ordinary AWS UID 10001 now passes supported `biorouter crew daemon start/status/stop` in an isolated profile: stable profile `ad9cf32c-82a5-4ad8-8f6c-78e4cf159201`, initial instance `e63d2deb…`, correct stop, then new instance `3e21cbd9…`. Wrong-proof stop returns 403 and status remains live; final correct stop succeeds. The temporary mode-600 secret is supplied only through stdin. All three users’ helper hashes match. Canonical GUI broker 4652 is separate and untouched. This qualifies the supported CLI lifecycle beyond the earlier plain-HTTP smoke; it does not establish GUI/CLI same-instance sharing.

The isolated CLI lifecycle receipt is recorded in `/private/tmp/crew-three-user-20260923T1606Z-luna/fixture-state.json`; no secret or public key is reproduced here. The current fixture retains its 18:30 UTC cleanup deadline under supervisor 17881, with teardown still pending.

### Three attached Crew views and source-only shutdown guard

The `main.ts` pre-ready shutdown guard now checks `app.isReady()` before `globalShortcut.unregisterAll`; independent Astra review found no issues. Luna reports two files/eight native-prompt/logger tests, Prettier and full `source bin/activate-hermit && CARGO_BUILD_JOBS=2 just check-everything` passing on dirty-source digest `553f376a35b3ee2a8e818d063c1ac1e687d342fee98cb3357246c730eb3bb010`. Running UI assets have not been rebuilt with this guard, so no guard runtime pass is claimed. Literal backslash-plus-n instead of a newline and a wrong `DEV_PROFILE_ROOT` were corrected QA setup errors, not product credential defects.

The current Alice/Bob/Carol attachment and CLI identity observations above are bounded setup/identity evidence. Alice bidirectional GUI/CLI message visibility now passes as recorded above; full three-user collaboration, native Save and detach/reopen remain unqualified. Secrets and screenshots remain local-only.

### Actual named native Save and retained-receipt cleanup failure

Current pushed source is `37a50626`, including guard `ad161959`. Alice’s actual GUI uploaded a valid 75-byte PNG (transfer `3e38a6a3f9d30debb568d1e9a68d770f`, request `9d72a548-8df0-4483-8edb-a560b093aeda`) and native image Preview succeeded. Native Save created `/Users/wgu/alice-native-saved-tiny-20260923.png`, mode 0600, 75 bytes, SHA-256 `9ccfc2abaa3984dc34c93aee16be0afa8a5e1395f25492b3df67897e6d00df10`, matching the original. This is the first bounded actual named-Save pass, not temporary staging alone.

One normal Forget attempt returned HTTP 400: `Failed to parse the request body as JSON: EOF while parsing a value at line 1 column 0`; the receipt remained. Pending source changes clear the JSON content-type header for absent bodies in `crewApi.ts` and both CLI HTTP paths. Independent review found no issues; Luna testing is in progress, with no fix/retest pass yet.

Bob and Carol passed supported native CLI authentication, enrollment, team acceptance and chat. Connections are `d783c17d-52be-4873-b884-b3f36ba2b72f` and `37a85c6d-918c-4eb1-9fc6-84e53846fb66`; principals are `369bddc2-13be-4d5c-b251-13367635e136` (UID 10002) and `1509377c-63ec-44ca-8a4e-c7cc7e0f45a8` (UID 10003). Team `ea5114dc-43ad-4a72-80bf-ce83d2bababd` has general channel `60dd49ff-fd85-4d0d-90b6-bceb1f0163e3`; posts are `55110f61-5f28-42b8-9c08-b2313133a7cb` and `75542704-38e0-467c-b112-b2d5625a9bdb`. Cross-user GUI visibility is still pending; no full three-user pass is inferred. Secrets and screenshots remain local-only.

### Three-user GUI message visibility and bounded fix progress

Bob GUI message `c7484ceb-67cd-43b6-9b2f-57760f52978d` has body `Bob GUI exchange 20260923T1718Z`; Carol GUI message `7faa6ecf-7d04-46a1-8d65-e5c0b3618410` has body `Carol GUI exchange 20260923T1718Z`. Both rendered across CDP 9222/9223/9224, and supported CLI history independently matched IDs, actors and channel. Sanitized receipt: `/private/tmp/crew-three-user-20260923T1606Z-luna/shared-membership-readonly.json`. This closes the bounded three-user messaging/visibility case, not full G11.

The empty-body header fix passes 17 daemon-client tests and six Crew frontend files/27 tests. The first full gate stopped on the request function’s 101-line Clippy threshold. Reusing the existing `bounded_response` helper preserves the same 180-second/16 MiB bound; independent review found no issues. The full gate now passes with exit 0 after helper reuse; the native pair is rebuilt. Actual Forget retest remains pending. The initial absolute-assets renderer was never installed; the corrected relative-assets build is verified but not installed, with GUI retest pending.

Alice’s native named-image Save retains its recorded pass. Bob’s attempt is inconclusive because the chooser interaction produced neither receipt nor file; no product Save defect is inferred. Alice’s owned task session `20260923_1` has remote.read/remote.write and a 21-byte `result.txt` containing `Row count: 3, Sum: 10`. Independent run/UID corroboration passes for this one owned read/write task; all-three-owner and remote.execute acceptance remain open.

### Final serialization artifacts and one independently verified owned task

After helper reuse, `just check-everything` passes with exit 0; daemon-client tests pass 17/17 and Crew frontend tests pass 27 across six files. The rebuilt pair at `/private/tmp/crew-observer-serialization-artifacts-20260923T000000Z` contains CLI SHA-256 `5ee21a9547e5e52b0248108370d3d7d746e28b85d94e8c4ba02fde45b75724e8` and unchanged daemon `0890a909cd1e6933ae6aa06549275e5d0152a72c006b21a7bb713de0c7ee2cf6`. Root verified relative `./assets` in `/private/tmp/crew-renderer-serialization-20260923`, index SHA-256 `bd574c95ebbd3a59d3584a4707b6a488c922fa7d3dbf9fa3f281e5e8edd6e6a6`. That renderer is not installed; no runtime Forget/cleanup retest pass is claimed.

Independent receipt `/private/tmp/crew-three-user-20260923T1606Z-luna/alice-owned-agent-readonly.json` verifies Alice run `23fdd6b5-ea38-4351-82c9-ca8d66a43b45`, session `20260923_1`, completed with null error and remote.read/remote.write tools. The input CSV is UID 10001, mode 0600, 36 bytes. Output `result.txt` is UID 10001, mode 0600, 21 bytes, SHA-256 `c8d64b8b6442dbf7f357474d23895634932ec16f5f7fe9797a179b9a1572b941`, exactly `Row count: 3, Sum: 10`. This is one owned read/write task pass, not all-three-owner or remote.execute acceptance.

### Installed renderer manual cleanup and task-result qualification — c4966613

Pushed `c4966613bf68fc9b36914b01a8ef106295e35bd9` includes the bodyless-header correction with 17 daemon-client tests, 27 Crew UI tests, full local gate and native CLI build passing. Corrected relative renderer index `bd574c95ebbd3a59d3584a4707b6a488c922fa7d3dbf9fa3f281e5e8edd6e6a6` was installed and all three clients reloaded. Actual manual GUI Forget of download receipt `d2cc7bbc14e98a31e449ed279ae35754` returned GET 200 then DELETE 200/`forgotten:true`; the receipt disappeared and the attachment remained available. This does not prove automatic receipt cleanup after a fresh upload/post.

Bob run `503a572c-201d-4161-9b23-2824d10074df` is completed but its actual attachments array is empty and `/home/crew_bob/crew-synthetic-work/result.txt` is absent. Markdown `/remote/path/result.txt` is not an output file; the requested objective failed. Independent receipt: `/private/tmp/crew-three-user-20260923T1606Z-luna/bob-run-and-carol-input-receipt.json`. Carol distinct-input task `b934abfe-f9e7-422e-8d4e-b17af998434c` is still running; no pass is claimed.

A pending four-file GUI resume-session fix allows nonblank `/pair?resumeSessionId` to resume its existing daemon session despite a missing global default, displaying the effective persisted binding in composer/chip. Independent Astra review found no issues. Luna reports 44 focused tests and two existing invalid-resume tests passing (101 skipped), plus typecheck/lint/format; fresh renderer/full gate and live retest remain underway. At 17:46, c496 hosted three-OS Rust/cross-Windows checks were pending and Linux package workflow 35896717730 was building. A cleanup deadline extension was requested but is not yet verified; the previous deadline remains authoritative until the AWS owner confirms it.

### 2026-09-23 follow-up: fresh upload, resume build and cleanup supervision

On the installed serialization renderer, Alice uploaded and posted the 75-byte `alice-second-tiny-20260923.png`, SHA-256 `9ccfc2abaa3984dc34c93aee16be0afa8a5e1395f25492b3df67897e6d00df10`. Message `e4ce967a-48f3-4f2a-a774-940257077bf8` carries attachment `d0a72f65-a815-4ea0-8a30-ee7b88e43751` with `restricted: true`. Alice and Bob rendered it and Bob’s supported CLI history matched. Its upload receipt disappeared automatically without an alert; the exact DELETE status was not captured. Carol’s visibility remains under investigation while her distinct-input task is running, so this is not three-client file acceptance.

The reviewed four-file resume-session change passes the full `just check-everything` gate. The relative-assets renderer at `/private/tmp/crew-renderer-parity-20260923` has index SHA-256 `26f075ca0997119c71f4c0ce0392eec5de8aab36f1fe0c92a06e5e4a88181cbb`; it was not installed as of 17:53 UTC, and live resume verification remains pending.

The AWS owner verified replacement cleanup supervision in `/private/tmp/crew-three-user-20260923T1606Z-luna/cleanup-supervisor-state.json`: prior session 17881 terminated cleanly; replacement foreground session 5196/PID 68689 enforces the absolute deadline `2026-09-23T20:30:00Z`. Cleanup has not started. Earlier deadlines remain historical checkpoints.

The parity renderer was subsequently installed, retaining a local backup. After Bob’s normal reload, Open agent session at `/pair?resumeSessionId=20260923_1` rendered the persisted transcript, qwen3:8b, tool progress and final result without a global-default write or daemon restart. Carol’s normal refresh initially showed a transient view; normal Reconnect restored Connected with verified identity, while her task remained running. These bounded observations do not qualify the full observer/recovery matrix.

### 2026-09-23 bbcd115c checkpoint and open runtime findings

Pushed source `bbcd115c7dfd7f2da1d4e5b2b96b45682957a663` includes the four-file GUI resume fix, its full gate and 44 focused plus two existing invalid-resume passes, and Bob’s installed live resume result. New server projection changes remain pending: source review identified serial/lazy tool polling; the proposed bounded concurrent projection now retains active wire exchanges for a bounded drain (at most 50 seconds), discards queued work, and handles non-fused EOF. Luna tests are in progress, without a runtime correction pass. Carol run `b934abfe-f9e7-422e-8d4e-b17af998434c` reached the existing 15-minute execution limit without an artifact; actual causality is unresolved.

Public GUI epoch 1: personal session `20260923_3` was initially reported to return `PUBLIC_PROVIDER_OK`, but that text may have been its generated title, so assistant output is unqualified. Initial and repeated fresh Private Crew requests returned HTTP 400 `Private cluster blocks public models`, without a run. That sink used a fixed wrong marker and count/hash, so it does not support an actual private-canary-byte claim. Corrected epoch 2 records actual marker flags without raw bodies: two POSTs at 18:05:24 both had the public marker and all private flags false. Count is two, not one. Reopening the second personal control (`20260923_5`) showed only the user prompt; its generated title contained `PUBLIC_PROVIDER_OK`. The sink returned JSON even when streaming was requested. A third epoch with valid SSE responses is being prepared; this is a fixture defect, not an established product failure. Same-fingerprint Private Crew replay returned HTTP 409 `crew_start_outcome_unknown` without a count increase; this is not a fresh HTTP 400 policy test. Replay UX assessment and a fresh unique private case remain pending. Local receipt: `/private/tmp/crew-three-user-20260923T1606Z-luna/public-sink/epoch2-final-observation.json`.

Verified c496 Linux artifacts at `/private/tmp/crew-linux-c4966613-ci/verified` are mode 0555 ELF, maximum GLIBC 2.30: CLI `e28a526fd867c2eec72d962212e99b793ad6c84dea61e3806aa248e5dc23b4d2`, daemon `f72b4753405d18af5affe27982e7a283036c75151961b4b627045d3364ea17a1`, broker `edc3133a8d80489b26497374d32472690179247b33394f0423ef7955fd81aac4`. Workflow 35896717730 backend succeeded; packaging was pending at the last check. These are artifact checks, not a c496 Linux runtime acceptance claim.

### 2026-09-23 epoch 3 privacy control and native binary Save

The corrected epoch 3 sink supports streaming SSE and ordinary JSON. In ungranted personal session `20260923_7`, the GUI rendered an actual assistant bubble `PUBLIC_PROVIDER_OK`. Two POSTs were recorded: `stream: true` SSE response, 132,108 bytes, SHA-256 `8dc5e1fb2b937c330a7009ac7b658700478d62d0d9071135206e1ab5750f8acc`; and `stream: false` JSON response, 441 bytes, SHA-256 `46568cbb5652046a5d8b0cc80877fa8d51f8503642a3339527bc39334f2c860b`. Both recorded the actual public marker and no private-marker flags. Fresh Private Crew request `5d59ec61-b4d8-43c9-92df-5fc9648954e9` used the distinct `_AFTER_CONTROL` canary and returned HTTP 400 `crew_request_refused` / `Private cluster blocks public models`, with null run ID. Sink count stayed at two; the private-prefix check covers that suffix. Local sanitized receipt: `/private/tmp/crew-three-user-20260923T1606Z-luna/public-sink/epoch3-final-receipt.json`. Epoch 1’s assistant claim is provisional because it may have been a generated title; epoch 2 supported history contained only a user prompt and title and qualifies dispatch only. Neither is upgraded by the corrected epoch 3 result.

Alice uploaded the 2,097,169-byte `/Users/wgu/crew-qa-binary-20260923.bin`, SHA-256 `e87d4e7122fbf266dab85ad7072c7305dcf3290d6a4ab623b06abc6d01625d42`: message `b478e6d3-64bd-49b1-80fc-24cf6eff4f1d`, blob `2f9acb53-4430-43d9-9173-1368fdb22ba0`, transfer `1ea2c6bdc7bb158756a00daf3842fb75`. Upload cleanup returned DELETE 200. Bob’s actual native Save to `/Users/wgu/crew-qa-bob-saved-binary-20260923.bin` matches the exact size/hash. A subsequent authoritative audit resolved the two posts as deliberate separate submissions. Broker record `192ee18b-b5c2-4417-b3b9-38141d862cc3` at recorded time 11:17:46 carries attachment `40aa3a37-e35f-4c3b-88f4-1b41b9c44cca`; record `b478e6d3-64bd-49b1-80fc-24cf6eff4f1d` at 11:19:02 carries attachment `2f9acb53-4430-43d9-9173-1368fdb22ba0`. After the transient view, the tester explicitly reopened the chooser, uploaded the fixture again, refilled the composer and submitted once. This is not current evidence of a product duplication defect. Request keys were not captured, so no automatic-retry or idempotency pass is claimed. Bob saved the second blob; completed download receipt `4a8ff7a9e6403a38170d0bd0f2053526` matches the size/hash.

Projection/preflight source review has no findings; 15 focused server tests and schema checks pass. A full-gate Clippy 105/100-line failure was addressed through pure identity-helper extraction; further preflight/EOF tests, full gate and build are running. No runtime projection-fix pass is claimed. Linux c496 workflow 35896717730 now fully succeeds. At 18:20 UTC, bbcd CI checks passed except Ubuntu, which remained in its hosted Clippy step. The cleanup deadline remains 20:30 UTC under supervisor PID 68689.

### 2026-09-23 projection/preflight local gate and artifact checkpoint

Luna reports full `just check-everything` exit 0 in session 15405, closing the earlier Clippy length failure after pure helper extraction. Focused projection/preflight checks pass; the renamed cancellation test passes 1/1. Additional exact selected-test counts are not yet recorded here. No runtime projection-fix acceptance is claimed.

The new native pair is `/private/tmp/crew-run-projection-native-artifacts-20260923T183329Z`: daemon SHA-256 `a6b4090473dc608371ce1a21f721cc7de349f532d452764cb36a66950463c381`, CLI SHA-256 `375383013f74f2d469f51f282cfc9a538ea08cb735f80a33d0cd917a31ac73d3`. Full Electron assets at `/private/tmp/crew-electron-complete-20260923T183654Z` contain 277 files (14 MiB) with relative asset paths; these assets are not installed and have no runtime qualification.

Root independently verified all required bbcd hosted checks, including Ubuntu, passing at 18:30 UTC. Linux c496 package workflow 35896717730 also fully passes. These results do not qualify later source changes or replace the outstanding installed-artifact acceptance.

### 2026-09-23 c9bbcfa4 upgrade and bounded private preflight replay

Product source checkpoint `c9bbcfa4` is pushed. All three profiles now use the previously recorded native pair: Alice daemon PID 27061, Bob 27419, Carol 27759, retaining private connections. Complete Electron assets were installed with local backup `/private/tmp/crew-electron-live-vite-backup-20260923T184630Z` and manifest `/private/tmp/crew-electron-daemon-handoff-manifest-20260923T184630Z`. Apps were not relaunched. The separately identified task-created Alice stock-Electron fallback PID 21692 was stopped exactly; all Crew clients were closed for CLI replay, while unrelated Electron remained untouched.

Supported native CLI connected Alice as UID 10001. Request `luna-private-preflight-custom-20260923` returned HTTP 400 `Private cluster blocks public models` twice with the identical request ID, no new run and sink count unchanged at two. Internal receipt absence was not directly inspected. Local receipt: `/private/tmp/crew-three-user-20260923T1606Z-luna/updated-native-private-preflight-receipt.json`. This is a bounded native replay result, not post-upgrade GUI qualification.

Earlier actual GUI ACL setup: Alice created restricted channel `qa-acl-20260923` (`6a1b1239-e37b-494d-860f-fe21cabb0c46`); Bob and Carol accepted actual UI invitations and all three selected the channel with three members. Non-owner management controls were absent. This does not establish authoritative server denial.

A subsequent source deadline correction is independently reviewed without findings but not yet test/build qualified. After the outer 900-second timeout, it retains and polls the pinned future for 55 seconds after cancellation to allow the internal 50-second projection drain to finish. Expiry remains the primary error; unsettled cleanup is explicitly unconfirmed. Rechecks after initial publication and before final publication prevent new work during the grace period. Tests, the full gate, rebuild and runtime evidence remain pending.

### 2026-09-23 deadline validation and authorized non-owner refusal

The deadline correction passes 19 Crew route tests and one core preflight test (4,250 filtered), independent review without findings, and full `just check-everything` exit 0 in session 9330. The rebuilt immutable pair is `/private/tmp/crew-deadline-native-artifacts-20260923T190049Z`: daemon SHA-256 `4af17abe9d233b559a368ab7a617a1f2ae6b0dd8064fcf01309046f6a059c6bf`, CLI `f216e05b9d918f0bb50ac19cb632bb250b02774fdfea2ec5bff4fbd89647c04a`. These checks close the preceding pending test/build checkpoint, without claiming a live deadline expiry pass.

Supported handoff preserves all three profiles: Alice daemon PID 48473/instance `fb519332-264e-4a91-bab9-67e78149da0e`, Bob 48846/`4c6b9219-a1a6-49dc-9a97-ddafe79cb8ad`, Carol 49212/`c4ca71d8-8157-4413-a794-08b8da086c54`, with 2/1/1 saved connections respectively, currently disconnected. Manifest: `/private/tmp/crew-deadline-native-handoff-manifest-20260923T190049Z.json`. GUI reopen is beginning; there is no new GUI acceptance result.

On the c9 pair, the non-owner test initially rejected by automatic review was subsequently explicitly approved by the user and executed. Bob’s remove-member request targeting Alice in `qa-acl-20260923` returned HTTP 400 containing `forbidden`/current-owner-required refusal. After normal reconnect, Alice’s independent supported readback confirmed all three active UIDs unchanged. Receipt: `/private/tmp/crew-three-user-20260923T1606Z-luna/bob-nonowner-permission-receipt-approved.json`. This closes that bounded authoritative-denial case; the earlier rejection is historical, not a continuing approval blocker.
