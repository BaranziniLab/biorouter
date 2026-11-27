# Stalled agents and results that lie

Two ways a long campaign loses hours without anything failing loudly: an agent that stopped
moving, and a check that reported success for work that failed or never ran.

## Stalled agents

- **What it looks like.** On 2026-10-23 two workflow agents in different workflows hung four
  minutes apart. Each had run a Bash command that finished on the machine, but the tool result
  never reached the agent, so its transcript stayed frozen on that tool call for two hours while
  the workflow waited. Other agents kept working, so it looked like a slow step.
- **Where to look.** A workflow run keeps `journal.jsonl` and one `agent-<id>.jsonl` transcript
  per agent under `~/.claude/projects/<project slug>/<session id>/subagents/workflows/<run id>/`.
  An agent is running when the journal has its `started` line and no `result`, `failed` or
  `skipped` line with the same `key`. Its idle time is the age of its transcript's mtime.
- **The watch.** `../scripts/stall-watch.py --session-dir <session dir> --run <run id>` prints a
  STALL line for each running agent idle longer than `--idle-min` (25 by default), with the last
  tool call it made and whether that call is still waiting for its result, then exits 5. Run it in
  the background beside long workflows. Pass `--run` for the runs you know are live: a run stopped
  with TaskStop leaves `started` lines with no result, which reads like a hang.
- **Long single steps are not stalls.** A cold build in a fresh worktree, a full test suite or a
  big deletion can sit quiet for 20 minutes. Check the process before calling it hung.

### What to do with a stall

1. Check the process is gone or idle: `ps` for the command the last tool call ran.
2. Read the transcript's last tool call (the STALL line shows it) and any uncommitted work the
   agent left: `git -C <worktree> status --short` and `git diff --stat`.
3. **Do not message the running workflow agent.** A SendMessage to one answered "Resuming agent"
   and started a second live copy that edited the same files as the first (2026-11-12). Both
   share one agent id, so you cannot address only one.
4. Stop the workflow, then relaunch only the unfinished items in a new, small workflow whose
   prompt tells the agent to continue from the uncommitted diff. **A workflow resume replays only
   the unchanged prefix of agent calls;** everything after the first unfinished call runs again,
   finished agents included.
5. Read every verdict joined to its agent's `started` label by `key`, never by position in the
   journal. Pairing a FAIL with the wrong label once merged a pull request that had failed.

## Results that lie

Each of these reported success for work that failed or had not finished. All of them fail in
the expensive direction: a false pass gets committed or shipped.

### Background tasks and pipes

- **`nohup ... &` inside a background Bash call double-backgrounds.** The harness tracks the
  launcher, which exits at once, so "completed (exit code 0)" arrives while the real work runs on.
  Pass the plain command with `run_in_background: true` and let the harness own it.
- **A pipe reports its last stage.** `cargo test ... | tail -200` returns tail's exit code; a
  failing target printed `error: 1 target failed` inside an exit-0 run. Use `set -o pipefail`, or
  write to a file, capture `rc=$?` right after the command and `exit $rc`.
- **`cmd | tee log` and `cmd; tail log` in a background task** report the last command's code.
- **Grep the work's own terminal marker** before believing a green notification: `test result:`,
  `Finished`, `^error: .* target`, or your own `rc=` echo.
- **`test result: ok. 0 passed`** prints as success when a filter matched nothing. Assert a
  non-zero count or run `--list` first.

### Waiters and monitors

- **A waiter that matches itself never exits.** `until ! pgrep -f "cargo test -p biorouter-mcp"`
  matches the waiter's own command line; eight such loops spun for hours. Use a character class
  the shell expands but the pattern does not match: `pgrep -f 'cargo test --workspac[e]'`.
- **A CI monitor that counts "completed" and excludes only "failure" scores a cancelled run as
  green.** Demand `conclusion == "success"` per job, and require one status per expected job so an
  empty or partial answer is not "done".
- **BSD grep reads `\|` in an extended pattern as alternation with an empty branch,** so
  `grep -qvE '\|completed\|'` matches every line. Split fields with `awk -F'\t'` instead.

### zsh, the Bash tool's shell

- **zsh does not word-split an unquoted `$var`.** `for pair in "a b"; do set -- $pair; ...` leaves
  `$2` empty; a merge-conflict check then reported every pair as conflicting, and once reported
  conflicting pairs as clean. Pass arguments explicitly, or run the loop under `bash -c` with a
  real array.
- **`$c:path` is a zsh history modifier** that eats the text after the colon. Write `${c}:path`.
- **Backticks or `${}` inside a double-quoted grep pattern** are expanded first. Use `grep -F` with
  single quotes.

### Checks that cannot fail

- A guard that matches its own explanatory comment, a gate that blanked most of the file it
  policed, a hash of the maker's own input, a verify step that prints a version and never
  compares it: each was written as the safeguard for a real trap and observed nothing.
- **Before trusting a check, make it fail once on purpose** and read the failure's message, not
  just its exit code: a negative control that fails for the wrong reason (a missing import) proves
  nothing. `../scripts/test-resource-guard.py` is this rule applied to the guard: it checks that
  each purge rule both deletes what it should and keeps what it must, and that a failed lsof stops
  the deletions.
- **A sampled audit list is a floor, not a total.** Fixing the lines a review listed and calling
  the category closed left a tail every time; run a detector over the whole surface.

### Failures that are the machine, not the code

- Two cargo runs in one worktree collide on the target lock and manufacture failures that vanish
  when run alone (16 knowledge tests on 2026-09-02). Run one build per worktree.
- Switching branches in a worktree while a background build runs there breaks it with errors that
  read like real defects (a missing test file, a lint you already fixed).
- Under heavy load the vitest browser test's `afterAll` overran its 30 s hook timeout on pristine
  `main` too. Note `uptime`, compare with a control run on `main`, and report runs verbatim.

## Related documentation

- `../SKILL.md`
- `processes.md` in this folder
- `docs/testing/continuous-integration.md` (what CI runs, and why a cancelled `main` run has no
  result of its own)
