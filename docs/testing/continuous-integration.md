# Continuous integration: what runs when

> **What this is.** The map of the GitHub Actions checks on this repository: which jobs run on a
> pull request, after a merge to `main`, nightly and on a release, how long each takes, and why
> the pull request set is smaller than the `main` set.
> **Status:** Current (layout of 2026-11-13). Times marked *measured* come from real runs; times
> marked *estimated* are derived from the step timings of the old layout and are replaced by
> measurements as runs accumulate.
> **Audience:** developers and agents who push pull requests, and maintainers who change
> `.github/workflows/`.

Until 2026-11-13 a pull request's CI took 60 to 90 minutes and often more: one `test` job per
OS ran the whole Rust workspace in sequence, and the macOS one alone took 58 minutes median.
Only two pull requests merged in five hours during a merge campaign, so the owner asked for pull
request CI under 20 minutes without dropping coverage silently. This page records the layout
that came out of that, what each job costs, and where every check that left the pull request
set runs now.

## Measured before the change

From 96 completed runs (22 Rust and 23 Frontend pull request runs among them), 2026-11-13. Full
tables: the orchestration notes `ci-timings-2026-11-13.md` outside the repository.

| Job (old layout) | Pull request run time, median / max (min) | Where the time went |
|---|---|---|
| `test (macos-latest)` | 57.8 / 90.3 | 45.5 min compiling lib and bin tests (7.7 on ubuntu: the 3-core, 7 GB arm64 runner is outgrown by the `biorouter` test crate), 10.6 running them, 8 min rebuilding 138 crates for a `-p biorouter` step |
| `test (ubuntu-latest)` | 43.4 / 63.2 | lib + bins 15.0, integration binaries 15.7, host clippy 4.7 (informational), Ollama 3.7, ubuntu socket gate 2.9, all in sequence |
| `test (windows-latest)` | 33.4 / 47.5 | lib + bins 21.0, Ollama 4.7, hostname clippy 3.8 |
| `Unit tests (vitest)` | 23.1 / 28.8 | one job running every vitest file |
| `cross-check (x86_64-pc-windows-gnu)` | 18.5 / 21.6 | check 9.4 and socket gate 8.1, in sequence; its cache restored nothing in 12 of 12 runs |
| `serve` | 11.6 / 21.1 | its own debug build of the CLI and daemon (5.3), plus a lifecycle test the integration step already ran |
| `cross-check (x86_64-unknown-linux-gnu)` | 9.3 / 10.5 | cold check every run |
| `Generated API contract` | 5.8 / 12.1 | its own build of the schema generator |

Queue waits added a median of 2 to 12 minutes per job, with peaks past an hour: the
organisation's plan runs 20 jobs at once, 5 of them on macOS.

## The layout now

### On every pull request

| Check | What it runs | Time (min) |
|---|---|---|
| `guards` | cross-drift, retired-name, version, Copilot naming, vendored source guards (Open Computer Use and whisper.cpp), the dictation fixture budget and build prerequisites | 0.5 *measured* |
| `cross-check (x86_64-pc-windows-gnu)` | `cargo check --workspace --all-targets` and the hostname-call clippy gate, in the release's docker image | ~14 *estimated* |
| `No inheritable sockets (x86_64-pc-windows-gnu)` | the socket gate on the target where the bug lives | ~9 *estimated* |
| `lib tests (ubuntu)` | `cargo test --workspace --lib` | ~15 *estimated* |
| `integration (ubuntu, 1/3)` | bin targets, Ollama readiness, sandbox integration, a third of `tests/*.rs` offline | ~13 *estimated* |
| `integration (ubuntu, 2/3)` | a third of `tests/*.rs` offline | ~12 *estimated* |
| `integration (ubuntu, 3/3)` | a third of `tests/*.rs` offline, the generated API contract, the `serve` browser contract, and served dictation in Chromium and Firefox (fake audio, a path prefix) | ~14 *estimated* |
| `test (ubuntu-latest)` | gate: every job above passed | seconds |
| `test (macos-latest)` | subset: Seatbelt sandbox, unit and integration | ~5 *estimated* |
| `test (windows-latest)` | subset: directory probes, sandbox unit, integration and `bounded_fs`, the console-window test | ~11 *estimated* |
| `vitest (N/3)` | a third of the vitest files each (`--shard=N/3`) | ~9 *estimated* |
| `Static checks`, `Registry generator`, `BAAM shelf (browser)`, `Preview panel (Electron)`, `Console windows (Windows)` | unchanged | 2 to 3 *measured* |
| `Unit tests (vitest)` | gate: the three vitest shards, `Static checks` and `Registry generator` passed | seconds |
| `no-ai-coauthor` | unchanged | 0.4 *measured* |

The slowest job sets the wall time: about 15 minutes plus the queue, against 77 minutes median
before. Path-filtered workflows (`Apps smoke`, `Rebuilt session storage acceptance`, `Biorouter
Copilot native payloads`) still run when their paths change and finish inside that.

**Why the ubuntu work splits this way.** Each job pays its own setup (about 2 minutes) and its
own compile of the workspace crates, so splitting buys less than it looks. The library tests are
one job because compiling them is one long chain (the `biorouter` test crate) whichever tests run
afterwards. The integration binaries are dealt round-robin by name across three shards, so a new
binary lands in a shard the day it is added, and each shard pays the library compile again but
runs a third of the 11 minutes of binaries. All four jobs build with `cargo test --workspace`,
never `-p`, so they resolve features alike and share one dependency cache.

**Why macOS and Windows run a subset.** Their full suites are the slow part of every old run, and
most of what they test is the same code ubuntu tests. What only they can prove is OS behaviour:
Seatbelt, Windows handles and ACLs, the PowerShell and console paths. The subset keeps the suites
whose subject is that behaviour, and `scripts/ci/choose-os-suite.sh` sends a pull request to the
full suite when its diff touches code specific to the OS:

- a `cfg` for the OS inside a changed hunk (the changed lines and three lines of context), not
  anywhere in the file, because shared files such as `session_manager.rs` carry a Windows `cfg`
  somewhere and a whole-file match sent most pull requests to the full suite;
- a code path under `crates/`, `scripts/` or `vendor/` that names the OS;
- the toolchain (`rust-toolchain.toml`, `.cargo/config.toml`), and on Windows the dependency set
  as well (`Cargo.lock`, any `Cargo.toml`), since a C or C++ dependency can build with mingw and
  fail with MSVC.

macOS gets no dependency rule because everyone who works on this repository builds and tests on a
Mac before pushing, so macOS code is compiled and run on every change; Windows code is not, which
is why Windows keeps the wider rule. Replayed against the open pull requests on 2026-11-13, the
dictation engine pull requests went to both full suites, the Crew coding parity pull request to
the full Windows suite (a Windows `cfg` in its change to `crew/authentication.rs`), and the
interface, test-fix and cost pull requests to the subsets.

### Every push to `main`, nightly, and on dispatch

Everything above, plus:

| Job or step | Why it is not on a pull request |
|---|---|
| full `test (macos-latest)`: lib + bins, Ollama readiness, "Backends declare macOS 12.0", the dictation engine, worker role and supervisor tests (Metal) | 58 min median on the scarcest runner; see the subset rule above. Also on a pull request labelled `ci:full` |
| full `test (windows-latest)`: lib + bins (with `--timings`), Ollama readiness, the dictation engine, worker role and supervisor tests (the MSVC engine build, M-3), the MSVC hostname clippy gate | 33 min median; the clippy gate runs on every pull request on windows-gnu. Also on `ci:full` |
| `cross-check (x86_64-unknown-linux-gnu)` (push and dispatch) | compiles what the ubuntu jobs already compile, in the release's bullseye image; the nightly full cross build covers that image too |
| ubuntu leg of the socket gate (in `lib tests (ubuntu)`) | the gate guards a Windows property; the Windows leg runs on every pull request |
| host `cargo clippy --workspace --all-targets` (in `lib tests (ubuntu)`) | informational: it fails only on a compile error, which the cross check and the test builds already catch |
| `cross-build-nightly` (nightly and dispatch) | the full release build of both cross targets, the glibc floor, the runtime-library contract, the dictation self test on bullseye, the old-processor check (DD-12), Windows system DLLs only, capture absent from the daemon (P-10); then `windows-native-dictation` runs the mingw binaries on real Windows under a folder name that is not ASCII (M-1) |

`main` runs therefore take about as long as before (the macOS full suite is still the longest
job), and a Windows or macOS failure that the subset could not see turns `main` red after the
merge rather than the pull request. The integration lane of a merge campaign, or whoever merged,
reads `main`'s run after each merge.

To get the full suites on a pull request, add the `ci:full` label and push again. GitHub reads a
pull request's labels from the event that started the run, so re-running an old run does not pick
the label up. Dispatching the `Rust` workflow on a branch also runs everything.

### On a release

Unchanged: `scripts/release.sh`, `release-artifact-smoke.yml`, and the GUI package workflows
(`linux-gui-packages.yml`, `windows-gui-packages.yml`, `computer-use-package-acceptance.yml`).
See [the release checklist](../releases/README.md).

## Required checks and the gate jobs

Branch protection on `main` requires five check names: `test (ubuntu-latest)`,
`test (macos-latest)`, `test (windows-latest)`, `Unit tests (vitest)` and `no-ai-coauthor`
(strict mode off). Renaming any of them blocks every merge, because a required check that never
reports leaves the pull request waiting forever.

Two of the five are now gate jobs rather than test jobs. `test (ubuntu-latest)` fails unless
`guards`, both cross jobs, `lib tests (ubuntu)` and all three integration shards passed;
`Unit tests (vitest)` fails unless the three vitest shards, `Static checks` and `Registry
generator` passed. Before, only the test jobs themselves were required, so a pull request with a
red retired-name guard, a red cross check or a type error could merge as soon as auto-merge saw
the five required names green. A gate uses `if: ${{ !cancelled() }}`, not `always()`, so a
cancelled run does not leave it queued holding a concurrency group, and it counts a job that a
`schedule` run skips by design as passing.

`BAAM shelf (browser)`, `Preview panel (Electron)` and `Console windows (Windows)` stay outside
the gate, as informational checks.

## Caches

The repository has a 10 GB cache ceiling, evicted least recently used, and on 2026-11-13 it held
9.47 GB, all written by `main`. Every writer saves from `main` only (`save-if`), because a
branch-scoped copy can only be read by its own branch and evicts the shared ones.

- `v0-rust-ubuntu-latest-test-…`: written by `lib tests (ubuntu)` on `main`, read by it and the
  three shards (`shared-key: ubuntu-latest-test`). The key is the one the old `test` job wrote,
  so the switch needed no cold start.
- `v0-rust-macos-latest-test-…` and `v0-rust-windows-latest-test-…`: the same keys as before,
  written by the full suites on `main`.
- The `serve` (2.0 GB) and `openapi-contract` (1.3 GB) entries are no longer written; GitHub
  drops them after a week unread, which leaves room under the ceiling.
- The cross-check jobs carry no cache. Theirs never survived to be read.

## Where things went

Nothing that guarded privacy or security left the pull request set: the privacy census and wiring
binaries, `privacy_ar15_is_retired`, the keyless-daemon binaries and the secret guard's tests run
in the ubuntu shards, the retired-name guard in `guards`, the socket gate and the hostname gate on
windows-gnu, and `no-ai-coauthor` as before.

| Was | Now |
|---|---|
| `serve` job | the serve steps of `integration (ubuntu, 3/3)`, using the executables that job's build made; its `serve_lifecycle` step was a duplicate of the same binary in the offline integration step and is gone |
| `Generated API contract` job (frontend.yml) | a step of `integration (ubuntu, 3/3)`, which runs `target/debug/generate_schema` instead of building it again |
| `cargo test -p biorouter --test ollama_*` | `cargo test --workspace --test ollama_*`, the same tests without the 124-crate rebuild |
| hostname clippy gate on MSVC, every pull request | windows-gnu on every pull request, MSVC on `main` and nightly |
| the cross-check cache step | removed (0 restores in 12 runs) |

## Changing the workflows

- A new OS-specific step belongs in the subset only if its subject is OS behaviour; otherwise put
  it under the full-suite condition (`steps.suite.outputs.full == 'true'`). A pull request that
  carries workflow steps written for the old matrix (`matrix.os == 'macos-latest'`, `runner.os ==
  'Windows'` inside the `test` job) has to move them into `test-macos` or `test-windows`.
- A new ubuntu test job reads the shared cache with `save-if: "false"`; only `lib-tests` writes
  it, so four jobs never race to write one key. Keep `cargo test --workspace`, never `-p`, or the
  job rebuilds the dependencies it should have restored.
- A new required check needs the owner's branch protection change. A job that only has to block a
  merge can join a gate's `needs` instead.
- vitest commands are read by `ui/desktop/src/test/vitestIsolation.test.ts`, which hands each one
  to vitest's own parser, so write literal flags (`--shard=2/3`), not `${{ }}` expressions.

## Related documentation

- [Process-global state in the Rust workspace](process-global-state.md): why some integration
  binaries must run in their own process
- [Debugging Biorouter on Windows](../desktop-ui/debugging-on-windows.md): reading a Windows CI
  failure from a Mac
- [How browser-served Biorouter is built](../deployment/serve-architecture.md): what the serve
  steps assert
- [How this documentation is organized](../organization.md)
