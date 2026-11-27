# Memory, CPU, heat and stray processes

The detail behind the memory, CPU and process rules in `../SKILL.md`, with the readings that
taught them. The owner's Mac has 16 cores and 128 GiB.

## Reading memory

| Signal | Command | Healthy | Act when |
|---|---|---|---|
| pressure level | `sysctl -n kern.memorystatus_vm_pressure_level` | 1 (normal) | 4 (critical) at once; 2 (warn) with sustained swapouts |
| free percentage | `memory_pressure -Q` | 50 % or more | it keeps falling while you add lanes |
| current paging | `vm_stat` Swapouts, sampled twice | 0 per second | hundreds per second for minutes |
| swap occupied | `sysctl -n vm.swapusage` | anything | never on its own |

- **Swap occupancy is history.** 18 to 30 GiB of swap stayed occupied for days after a large R job
  while pressure was normal and nothing paged. It does not prove current paging and is not a
  reason to stop work. Small swapin bursts with no swapouts are not pressure either.
- **What to do under real pressure:** stop starting lanes, let running ones finish, and pause
  your own heaviest job by pid (`kill -STOP <pid>`, later `kill -CONT <pid>`). Never stop the
  owner's applications or another session's work; ask.
- **Open files are a resource too.** A test that fails with a reqwest client builder `I/O error`
  before any request was a load artefact (44,140 open files at the time, 2026-09-03), not a code
  bug. The guard logs `open_files_pct` (`kern.num_files` over `kern.maxfiles`).

## Reading CPU and load

- **Load average is a poor gauge here.** The endpoint security agent, Spotlight and the
  file-provider service have held it at 40 to 60 with nothing of yours running (2026-09), and they
  use one to two cores between them; on a quiet day in late 2026-11 it sat near 5. Load 500 to 580
  on 16 cores with 24 lanes compiling was expected and not a fault (2026-11-20).
- **Read CPU idle instead:** `iostat -c 2 -w 1 -n 0` (the last line's `id` column). Under 10 %
  idle for long stretches means more lanes will not finish sooner.
- **Find who burns it:** `ps -axo pid,ppid,etime,pcpu,rss,comm | sort -k4 -nr | head -25`. Look
  for a process that is yours or a leak before blaming the agents.
- **CPU is usually the ceiling, not memory or disk.** More parallel cargo jobs than cores did not
  speed anything up; the release Docker VM already uses all 16 cores.
- **The concurrency stressor in this test suite is `RUST_TEST_THREADS`, not load.** Do not quote a
  busy machine as a harsh test condition.

## Heat

- `pmset -g therm`. On Apple Silicon the healthy output is three lines starting
  `Note: No thermal warning level`, `No performance warning level`, `No CPU power status`. A
  `Thermal warning level` line, or a `CPU_Speed_Limit` under 100, is a warning.
- On a warning: stop adding work, pause your heaviest job by pid, and tell the owner if it stays
  for more than a few minutes. No thermal warning was recorded in any 2026-11 campaign, even at
  load 580.

## Orphaned and runaway processes

- **The signature of a leak is parent pid 1 plus sustained CPU on a shell or interpreter.** On
  2026-07-30 a flakiness harness started `(while :; do :; done) &` loops and died before its
  `kill`; 24 loops ran orphaned for 4 h 42 min at about 42 % CPU each and load hit 500. Agents
  stalled and builds failed in ways that read like app bugs.
- Find them: `ps -eo pid,ppid,pcpu,etime,comm | awk '$2==1 && $3>20'`. System daemons (the window
  server, media analysis, the security agent, file provider) also have parent pid 1 and real CPU;
  the guard ignores anything not named like a shell, Python, node, vite, esbuild or cargo.
- **Never leave a load generator's cleanup to one `kill` at the end of a script.** Bound each
  burner with a deadline it enforces itself; macOS has no `timeout` command.
- **Playwright and e2e runs leak vite and esbuild dev servers** when teardown throws before the
  kill. After any e2e run: `ps -eo pid,ppid,command | awk '$2==1' | grep -Ei 'vite|esbuild'`, then
  stop the ones whose worktree is gone or is yours, by pid.
- **A process that is not yours: ask.** On 2026-11-20 a System Settings storage-size job
  (`ApplicationsStorageExtension`) had run 23 hours using up to 1.5 cores. The agent asked, the
  owner said yes, and two sibling processes from the same session had to go too. macOS restarts
  them when the Storage pane opens again.
- **Never stop** the owner's apps, R sessions, rsync jobs, browsers or the security agent, even
  when they are the top CPU users.

## Killing safely

- **Kill only pids you started, by pid.** Record the pid when you spawn (`$!`, or the Popen pid).
- **Never `pkill -f` or `killall` by pattern.** `pkill -f "target/debug/biorouterd agent"` matches
  every worktree's dev daemon, because the shared part of the path is the suffix. It killed other
  sessions' daemons on 2026-09-04 and twice on 2026-09-08 (once by a read-only investigator).
  Another session's daemon has its own ephemeral port, secret and data root, so you cannot restart
  it; you can only report it.
- **If you must match, scope it and read it first:** `pgrep -fa "$PWD/target/debug/biorouterd agent"`,
  then kill the listed pids that are yours.
- **Put this sentence in every subagent brief that can start a process:** "NEVER kill processes by
  pattern; kill only pids you started, by pid."
- **Freeze rather than kill** when a runaway build of yours is filling the disk:
  `kill -STOP <cargo pid>` stops the whole build without losing it; `kill -CONT <cargo pid>` after
  you free space.
- **A watchdog's pid file is a claim.** Before acting on one, check the process:
  `ps -o pid,lstart,command -p <pid>` must show the expected command and start time.

## Related documentation

- `../SKILL.md`
- `disk.md` and `stalls-and-false-greens.md` in this folder
- `docs/desktop-ui/launching-the-dev-gui.md` (dev GUI processes and the five launcher traps)
