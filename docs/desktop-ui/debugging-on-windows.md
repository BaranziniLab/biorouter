# Debugging Biorouter on Windows

> **What this is.** How to diagnose a Windows-only failure in this repo when you are
> almost certainly working on a Mac, plus the currently-open Windows defects and what
> has already been ruled out for each.
> **Status:** Current. The open-defect section is a snapshot of 2026-08-20 and is
> expected to shrink; the technique sections are durable.
> **Audience:** developers and agents debugging a Windows CI failure or a Windows-only
> bug report.

Nobody on this project develops on Windows, so every Windows problem is diagnosed at
one remove. That changes the method: you cannot bisect by running the thing. What you
have instead is CI, a cross-compiler, and the discipline of reading evidence rather
than guessing — and this page is mostly about not wasting a day guessing.

## Start by asking whether it failed or hung

**This is the first question, and it is easy to get wrong.** A hung Windows job looks
like a slow one, and "CI is slow tonight" is a story that fits the evidence and is
wrong. It cost several hours on 2026-08-20.

The tell is not the absolute time. It is the time **compared with the job's own
history**:

```bash
gh run list --workflow rust.yml --limit 25 --json databaseId,headSha,conclusion,status
gh run view <run-id> --json jobs   # read startedAt / completedAt per job
```

On this repo `test (windows-latest)` finishes in **20–41 minutes**. When it sat at 120+
minutes it was hanging, and it had been hanging on every merged head — while each
contributing branch had passed Windows individually in about 20. A defect that appears
only after a merge is invisible to every branch's own CI, which is exactly why the
duration comparison is worth doing before anything else.

A hang is worse than a failure in one specific way: **it hides every test after it.**
Fixing the hang on 2026-08-20 immediately revealed five real failures that had never
once been observed.

### Why a Windows test hangs rather than fails

The usual cause is a child process that never answers, awaited without a timeout.
Two ingredients, both present here:

- **A test spawns something Windows does not have.** `python3` is the recurring one:
  there is no `setup-python` step in [`.github/workflows/rust.yml`](../../.github/workflows/rust.yml),
  so on the Windows runner `python3` resolves to the Microsoft Store's App Execution
  Alias, or to nothing at all.
- **The await has no deadline.** `AppServer::request`
  (`crates/biorouter/src/providers/coding_agent/appserver.rs`) documents that it has no
  timeout of its own, and a real turn has none by default either: it ends when the child
  finishes or the user cancels, and the provider applies a wall clock only when an
  operator sets `BIOROUTER_CODING_AGENT_TURN_TIMEOUT_SECS`. A test that calls
  `request()` directly therefore waits forever.

So a missing interpreter does not fail the job. It hangs it to the runner's ceiling.
Tests that depend on a POSIX-shaped fake process are marked `#[cfg(unix)]` for this
reason; if you add one, mark it, or give it a timeout.

## What you can actually run from a Mac

| You want to check | Command | Catches |
|---|---|---|
| It still compiles for Windows | `cargo check --target x86_64-pc-windows-gnu` | type errors, `cfg` mistakes, dangling references after gating |
| It compiles including tests | add `--tests` to the above | a helper you gated but whose caller you did not |
| It links and packages | `scripts/release.sh windows <ver>` (Docker) | the mingw link fixes below |
| It runs | **nothing local — only CI** | everything else |

`cargo check --target x86_64-pc-windows-gnu` needs mingw on the host and will die with
`failed to find tool "x86_64-w64-mingw32-gcc"` without it. The release path avoids this
by cross-compiling in Docker; for a quick syntax/`cfg` check, prefer pushing to a branch
and reading CI's own `cross-check (x86_64-pc-windows-gnu)` job, which is fast (~8 min)
and runs on every push.

**Reading a failure out of CI:**

```bash
gh run view <run-id> --log-failed | grep -iE "panicked|FAILED|assertion|error\[" | head -40
```

`--log-failed` works only once the job has finished. There is no way to read a live log
for an in-progress job, which is another reason a hang is expensive.

## Windows traps that have actually bitten this repo

- **A file that is open cannot be deleted.** This is the single biggest behavioural
  difference from macOS and Linux, and it is the root of the ephemeral-store defect
  below. Anything that creates a directory, opens a handle into it (a SQLite pool
  especially), and then removes the directory needs the handle closed *first* —
  retrying the delete does not help, because the handle is still open.
- **Never route an absolute path through `cmd.exe`** (noted in `CLAUDE.md`; it was one
  of four defects an adversarial review found).
- **`HOME` is not defined.** Windows uses `USERPROFILE`. A bare
  `env::var("HOME").expect(...)` panics on any runner without a Git Bash session;
  `crates/biorouter-server/src/test_sandbox.rs` reads both names and documents why.
- **Cross-compile link fixes** live in the Justfile and `scripts/release.sh`:
  `aws-lc-sys` needs winpthread appended *after* the rlibs on the mingw link line, and
  `lzma-sys` needs `LZMA_API_STATIC=1`. Run the Docker cross builds with the system
  Docker — hermit does not shadow it.
- **After any Windows or Linux Docker build**, `ui/desktop/node_modules` is
  Linux-flavoured. Restore it with `npm ci` — **never `npm install`**, which rewrites
  the lockfile and breaks the next cross build.
- **Secrets are chunked** across multiple Windows Credential Manager entries (2560-byte
  cap each); see [secret storage](../security/secret-storage.md).
- **Windows has no in-place updater.** It ships as a plain zip and uses the assisted
  download fallback, so an auto-update problem reported on Windows is a different code
  path from macOS.

## Publication is gated on a native Windows smoke run

Nothing in the release pipeline executes the Windows build *on Windows* — the zip is
cross-compiled and packaged in Docker. That gap is why `scripts/release.sh publish`
refuses unless `release-artifact-smoke.yml` has a successful run titled
`Release artifact smoke v{ver}`. Treat that run as the only evidence the Windows
artifact actually starts.

## Open Windows defects

Snapshot of 2026-08-20, on `main` at `efe6cb41`. All five were invisible until the hang
above was fixed, so none of them is a fresh regression — they are newly *visible*.

### Four bridge tests: a PreToolUse hook that cannot speak `cmd.exe`

```
providers::coding_agent::bridge::tests::a_pretooluse_rewrite_decides_what_a_bridged_call_runs
providers::coding_agent::bridge::tests::a_rewritten_call_is_re_judged_by_the_security_floor
providers::coding_agent::bridge::tests::a_bridged_call_leaves_another_calls_rewrite_staged
providers::coding_agent::bridge::tests::concurrent_bridged_calls_each_run_their_own_rewrite
```

**One root cause, not four.** It is in the test fixture, not in the product.

`hooks_returning` (`crates/biorouter/src/providers/coding_agent/bridge.rs`) builds a
real hook — deliberately, because the rewrite is staged as a side effect of the hook
*running*, so a stub would test the assertion rather than the path. The command it
builds is:

```rust
// Single-quoted for `sh -c`, with any embedded quote escaped the usual way.
let quoted = payload.replace('\'', "'\"'\"'");
... format!("echo '{quoted}'")
```

That is POSIX shell quoting. `hooks/command_runner.rs` correctly runs hooks through
`cmd.exe /D /S /C` on Windows and `sh -c` elsewhere — the executor is right; the
**fixture** assumes the Unix branch. `cmd.exe` does not treat `'` as a quote
character, so it echoes the single quotes literally and the hook's stdout becomes

```
'{"hookSpecificOutput":{"updatedInput":{ ... }}}'
```

which is not valid JSON. The manager parses hook stdout to find
`hookSpecificOutput.updatedInput`, the parse fails, **no rewrite is staged**, and the
call dispatches the model's original arguments.

That single failure produces all four symptoms, which is why they look unrelated:

| Test | What you see | Why |
|---|---|---|
| `a_pretooluse_rewrite_decides…` | got `CFTR`, expected `TP53` | no rewrite, so the model's chromosome-7 query ran |
| `a_bridged_call_leaves_another…` | got `CFTR` | same |
| `concurrent_bridged_calls…` | got `CFTR`, message blames a sibling draining the rewrite | same — the message is a red herring here |
| `a_rewritten_call_is_re_judged…` | `-32002: Tool 'developer__shell' not found` | no rewrite, so the security floor saw a harmless `ls` and approved it; dispatch then reached for `developer__shell`, which `GeneFixture` never registers |

**Ruled out, with evidence** — do not re-derive these:

- *The `shell` tool is platform-gated.* It is not: `#[tool(name = "shell", …)]` in
  `crates/biorouter-mcp/src/developer/rmcp_developer.rs` is unconditional.
- *Builtin extensions spawn a subprocess that fails to resolve on Windows.* They do
  not — `ExtensionConfig::Builtin` loads in-process over `tokio::io::duplex` pipes.
- *A tool-discovery race.* This was the earlier leading hypothesis and it is **wrong**;
  the fourth test's fixture simply never registers that tool, and only reaches for it
  because the rewrite failed first.
- *A product defect.* The shipped hook executor branches correctly per platform. A
  Windows user writing a hook writes it in `cmd`/PowerShell syntax and it works.

**The fix is in the fixture**: emit valid JSON under both shells. The payloads in use
contain no `cmd.exe` metacharacters (`> < | &`), so an unquoted `echo {json}` is
viable on the Windows branch, with the existing single-quoted form kept for `sh`.
`#[cfg(unix)]`-ing the four tests also works but costs the only Windows coverage of
the bridge's rewrite path, which is where the bug would actually matter.

⚠ **This has not been executed on Windows.** It is derived from the source and from
the CI panic messages. Confirm on the box before trusting the `cmd.exe` echo
behaviour.

### `close_ephemeral_store_removes_the_directory`

`crates/biorouter-cli/src/session/builder.rs:1189` — after `close_ephemeral_store`, the
`biorouter-no-session-*` directory still exists on Windows, so an early exit leaks it.

This is the Windows half of the open-handle rule above, and it is **not fixed**. Note
for anyone reading older session notes: this fix was once reported as confirmed by CI.
It was not — the Windows job was hanging on every merged head at the time and never
confirmed anything. Treat that earlier claim as withdrawn.

⚠ **It is FLAKY, not deterministic, and that changes how you chase it.** On
`efe6cb41` it failed; on `858d9aaf` — with the CLI crate untouched between the two —
the same test passed on Windows (`1395 passed; 0 failed`). So a single green run
proves nothing here, and a single red one is not a regression you just introduced.
Loop it: `cargo test -p biorouter-cli --lib close_ephemeral_store` twenty times and
count, before and after any change. The underlying race is an open handle losing to
the directory removal, so it will correlate with machine load.

Related but distinct: on a developer machine `cargo test -p biorouter-cli` deadlocks
against a populated `~/.config/biorouter` (real knowledge bases, ~21 lock files) and
passes in 17 s against an empty one. To run that crate's tests locally at all, isolate
`HOME`, `XDG_CONFIG_HOME` and `BIOROUTER_PATH_ROOT` into a temp directory. CI never
sees it, because CI's config directory is empty. **That is a different bug from this
one** — same file, different cause — so do not conflate them.

## A checklist for the next Windows failure

1. Compare the job duration with its history. Hang or failure?
2. If it hung, find the child process that never answered, and the await with no
   deadline.
3. `gh run view <id> --log-failed` and read the *first* failure, not the loudest.
4. Ask whether the test has ever passed on Windows: `git log -S<test-name>` against the
   file, then check whether that commit's CI ever completed a Windows job.
5. Reproduce what you can on a Mac with `--target x86_64-pc-windows-gnu`, and be honest
   that it only proves compilation.
6. Change one thing, push to a branch, read CI. Resist fixing two things at once —
   Windows feedback costs 15–20 minutes a cycle and a combined change teaches you
   nothing.

## Related documentation

- [Launching the dev GUI](launching-the-dev-gui.md) — the equivalent procedure for
  driving the app locally, and the six ways it fails silently
- [Startup freeze and main-thread blocking](startup-freeze-and-main-thread-blocking.md)
  — the bundled MinGit copy was a Windows-first symptom
- [Window scaling regressions](window-scaling-regressions.md)
- [Secret storage](../security/secret-storage.md) — Windows Credential Manager chunking
- [Release process](../../RELEASE.md) and the Windows smoke gate in `CLAUDE.md`
