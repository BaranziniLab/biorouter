# Continuous integration: what runs when

> **What this is.** The map of the GitHub Actions checks on this repository: which jobs run on a
> pull request, after a merge to `main`, nightly and on a release, how long each takes, and why
> the pull request set is smaller than the `main` set.
> **Status:** Current (layout of 2026-11-13; superseded `main` runs cancelled since 2026-11-14).
> Times are measured on the pull request run that
> introduced the layout (#498, head f536dcb7e) unless marked otherwise.
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

| Check | What it runs | Run time (min) |
|---|---|---|
| `guards` | cross-drift, retired-name, version, Copilot naming, vendored source guards | 0.5 |
| `cross-check (x86_64-pc-windows-gnu)` | `cargo check --workspace --all-targets` (8.9) and the hostname-call clippy gate (4.1), in the release's docker image | 13.2 |
| `No inheritable sockets (x86_64-pc-windows-gnu)` | the socket gate on the target where the bug lives | 8.1 |
| `lib tests (ubuntu)` | `cargo test --workspace --lib` (9.1) | 10.5 |
| `integration (ubuntu, 1/3)` | bin targets (5.5), Ollama readiness, sandbox integration, a third of `tests/*.rs` offline (5.5) | 12.9 |
| `integration (ubuntu, 2/3)` | a third of `tests/*.rs` offline | 9.8 |
| `integration (ubuntu, 3/3)` | a third of `tests/*.rs` offline (7.6), the generated API contract, the `serve` browser contract | 10.0 |
| `test (ubuntu-latest)` | gate: every job above passed | 0.1 |
| `test (macos-latest)` | subset: Seatbelt sandbox, unit and integration | 1.6 |
| `test (windows-latest)` | subset: directory probes, sandbox unit, integration and `bounded_fs`, the console-window test (4.3) | 7.3 |
| `vitest (N/3)` | a third of the vitest files each (`--shard=N/3`) | 10.0 to 11.3 |
| `Static checks` | typecheck, eslint, theme, contrast, token gates, plus the registry generator and documentation checks (one job since 2026-11-13; they were measured at 2.4 and 2.9 apart) | about 4 *estimated* |
| `BAAM shelf (browser)`, `Preview panel (Electron)`, `Console windows (Windows)` | unchanged | 1.4 to 2.5 |
| `Unit tests (vitest)` | gate: the three vitest shards and `Static checks` passed | 0.1 |
| `no-ai-coauthor` | unchanged | 0.4 |

The slowest job sets the floor: about 13.5 minutes for Rust and 13 for Frontend when every job
starts at once, against 77 minutes median before. The run that measured these took 29 minutes
wall for Rust anyway, because its jobs waited 9 to 17 minutes for a runner (see "Runner slots"
below). Path-filtered workflows (`Apps smoke`, `Rebuilt session storage acceptance`, `Biorouter
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

Everything above, with the integration binaries in ONE job (`integration (ubuntu, 1/1)`, every
step of the three shards) and vitest in one job (`vitest (all files)`), plus:

| Job or step | Why it is not on a pull request |
|---|---|
| full `test (macos-latest)`: lib + bins, Ollama readiness | 58 min median on the scarcest runner; see the subset rule above. Also on a pull request labelled `ci:full` |
| full `test (windows-latest)`: lib + bins (with `--timings`), Ollama readiness, the MSVC hostname clippy gate | 33 min median; the clippy gate runs on every pull request on windows-gnu. Also on `ci:full` |
| `cross-check (x86_64-unknown-linux-gnu)` (push and dispatch) | compiles what the ubuntu jobs already compile, in the release's bullseye image; the nightly full cross build covers that image too |
| ubuntu leg of the socket gate (in `lib tests (ubuntu)`) | the gate guards a Windows property; the Windows leg runs on every pull request |
| host `cargo clippy --workspace --all-targets` (in `lib tests (ubuntu)`) | informational: it fails only on a compile error, which the cross check and the test builds already catch |
| `cross-build-nightly` (nightly and dispatch) | unchanged: the full release build of both cross targets, the glibc floor, the runtime-library contract |

Sharding is for a pull request, where someone waits; on `main` three shards would only hold
three runner slots. `main` runs therefore take about as long as before (the macOS full suite is
still the longest job), and a Windows or macOS failure that the subset could not see turns `main` red after the
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
`guards`, both cross jobs, `lib tests (ubuntu)` and every integration job passed;
`Unit tests (vitest)` fails unless every vitest job and `Static checks` (which carries the
registry generator checks) passed. Before, only the test jobs themselves were required, so a pull request with a
red retired-name guard, a red cross check or a type error could merge as soon as auto-merge saw
the five required names green. A gate uses `if: ${{ !cancelled() }}`, not `always()`, so a
cancelled run does not leave it queued holding a concurrency group, and it counts a job that a
`schedule` run skips by design as passing.

`BAAM shelf (browser)`, `Preview panel (Electron)` and `Console windows (Windows)` stay outside
the gate, as informational checks.

## Runner slots

The organisation's plan runs at most 20 jobs at once across all its workflows, 5 of them on macOS,
and a job that finds no free slot waits in the queue. The first run of this layout (#498's own,
2026-11-13 23:18 to 23:48 UTC) found the repository at exactly 20 running jobs in every
two-minute sample for its first 20 minutes, most of them belonging to `main`-branch runs still on
the old layout; its ubuntu jobs waited 9 to 17 minutes to start. So past a point, splitting a run
into more jobs makes it slower, not faster, and the cheapest minutes to win are slots:

- `main` and nightly run the integration binaries and vitest as one job each rather than shards.
- The registry generator checks run inside `Static checks` rather than in a job of their own.
- `no-ai-coauthor` runs on pull requests and on pushes to `main`, not also on every push to a
  pull request's branch, where it answered the same question twice.

- A newer push to `main` cancels the older `main` run (below).

A pull request's run holds 18 jobs (10 Rust, 8 Frontend) and a `main` run 15 (9 and 6), the
latter's macOS and Windows full suites holding their slots for about an hour and half an hour.

## A newer push to `main` cancels the older run

Since 2026-11-14, by the owner's ruling, the workflows that test every commit (`Rust`,
`Frontend`, `Apps smoke`, `Rebuilt session storage acceptance`) cancel their older `main` run,
running or still queued, when a newer push to `main` starts one, exactly as a pull request's new
push already cancelled its older run. During a burst of merges each push used to start a full run
that nothing cancelled, and those runs held most of the organisation's 20 slots while pull
requests queued behind them.

The cost: **an intermediate `main` commit can end up with no CI result of its own.** When `main`
turns red after several merges, the newest run says something broke but not which merge broke
it. To find out, run CI on the earlier commits. Do not re-run the cancelled `main` run itself: a
re-run joins the same `main` concurrency group and cancels whatever `main` run is in progress,
and the next push cancels it in turn. Dispatch on a throwaway branch instead; a dispatched run is
keyed by its own run id, so it cancels nothing and nothing cancels it:

```bash
sha=<the earlier commit>
git push origin "${sha}:refs/heads/ci-bisect-${sha:0:9}"
gh workflow run rust.yml --repo BaranziniLab/biorouter --ref "ci-bisect-${sha:0:9}"
```

(`"${sha}:refs/..."` with braces: in zsh `$sha:r` is a path modifier.) A dispatch runs every job,
the full macOS and Windows suites included. Delete the branch afterwards.

What is never cancelled by a push: the nightly schedule, a manual dispatch, the release and
package workflows (`release-artifact-smoke.yml`, the GUI package workflows), and the website
deploy (`deploy-landing.yml`, its own `pages` group).

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
- `main`'s single integration job reads the shared ubuntu key without writing it, like the shards.

## Where things went

Nothing that guarded privacy or security left the pull request set: the privacy census and wiring
binaries, `privacy_ar15_is_retired`, the keyless-daemon binaries and the secret guard's tests run
in the ubuntu shards, the retired-name guard in `guards`, the socket gate and the hostname gate on
windows-gnu, and `no-ai-coauthor` as before.

| Was | Now |
|---|---|
| `serve` job | the serve steps of `integration (ubuntu, 3/3)`, using the executables that job's build made; its `serve_lifecycle` step was a duplicate of the same binary in the offline integration step and is gone |
| `Generated API contract` job (frontend.yml) | a step of `integration (ubuntu, 3/3)`, which runs `target/debug/generate_schema` instead of building it again |
| `Registry generator` job (frontend.yml) | steps of `Static checks`, run from the repository root |
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
