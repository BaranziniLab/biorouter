---
name: resource-watchdogs
description: Watch and protect the owner's Mac during long multi-agent Biorouter campaigns, covering disk space (tiered cache purges, the never-delete list, cold storage as verified tars), memory pressure and swap, CPU load and heat, orphaned or runaway processes, stalled agents and background results that lie. Use it when you start, take over or hand off a campaign that builds or tests in parallel, before deleting anything to free space, and when builds, tests or agents fail or stall in ways that could be the machine rather than the code.
---

# Resource watchdogs

How to keep the owner's Mac (16 cores, 128 GiB, one internal data volume) healthy while many
agents build at once, and what you may clean up without asking. The scripts in `scripts/` do the
watching. The files in `reference/` hold the detail and the measurements behind each rule.

## Rules

1. **Run one guard per machine, started before the first build.** Six lanes took free disk from
   173 to 67 GiB in 45 minutes (2026-11-12); a guard started late finds the disk already full.
2. **Watch read-only first, then delete only what the tier table lists.** The owner's standing
   approval (2026-11-12, widened 2026-11-17) covers rebuildable build caches that no running build
   uses, and nothing else.
3. **Fail closed.** When you cannot tell whether a build uses a file (lsof or ps failed, a
   protect list is unreadable), leave it. The retired 2026-11 deleting watchdog failed open.
4. **Never kill by pattern; kill only pids you started, by pid.** `pkill -f` killed other
   sessions' dev daemons three times, and their ports and secrets cannot be recovered.
5. **Predict reclaim from `df`, never from `du`.** APFS clones made `du` overstate one cleanup
   4.5 times (689 GiB counted, 151 GiB freed).
6. **Load is not a verdict.** Security and indexing services alone have held it at 40 to 60, and
   24 compiling lanes took it to 580 with memory normal and no heat. Read memory pressure,
   current paging, CPU idle and the top CPU users instead.
7. **Swap occupancy is history; swapouts per second are the signal.** 18 to 30 GiB of swap stayed
   occupied for days while pressure was normal and nothing paged.
8. **An exit code of 0 is not the work's result.** Background launchers, pipes and cancelled CI
   all report success for work that failed or never ran. Check the work's own last line.
9. **A hung agent looks like a slow one.** Watch transcript idle time; a lost tool result froze
   two agents for two hours (2026-10-23).
10. **Ask the owner before touching what is not yours or not rebuildable:** other sessions'
    worktrees, targets and daemons, the owner's main checkout, system services and apps, Docker
    volumes, local models, macOS privacy settings.
11. **Use the machine fully while it is healthy.** The owner wants builds as fast as the machine
    allows: add lanes while there is disk room, pressure is normal, nothing pages and nothing is
    hot. CPU is usually the ceiling: 24 lanes ran at load 500 and more jobs would not have helped.
12. **Make a check fail once before trusting it.** A `pgrep` waiter that matched itself never
    exited, and a monitor scored a cancelled CI run as green. `scripts/test-resource-guard.py` is
    the guard's own negative control.

## Start the guard

Copy the scripts into your campaign folder first, so removing a worktree never removes the guard:

```bash
mkdir -p ~/biorouter-runs/<campaign>
cp -R <worktree>/.claude/skills/resource-watchdogs/scripts ~/biorouter-runs/<campaign>/guard
G=~/biorouter-runs/<campaign>/guard
python3 -I $G/test-resource-guard.py        # proves every purge rule on fake targets, about 10 s
python3 -I $G/resource-guard.py --once      # one read-only reading of the machine
python3 -I $G/resource-guard.py --once --purge --dry-run \
  --root ~/biorouter-runs --root <repo>/.claude/worktrees    # what a purge pass would take now
```

Then start it with the Bash tool's `run_in_background: true`, passing the plain command (never
`nohup` or `&`, which hand the harness a launcher that exits at once):

```bash
python3 -I ~/biorouter-runs/<campaign>/guard/resource-guard.py --purge \
  --root ~/biorouter-runs --root <repo>/.claude/worktrees \
  --protect-file ~/biorouter-runs/<campaign>/protect.txt
```

- **Roots** are where it looks for cargo targets. Give worktree folders and campaign folders,
  never the owner's main checkout.
- **The protect file** holds one path per line that the guard never touches at all: release
  worktrees, pinned binary folders, immutable stages, evidence, any lane you want kept warm, and
  any lane whose own handoff forbids deleting its caches. It is re-read every pass; an
  unreadable protect file stops all deletion.
- **On an alert it prints one `ALERT` line and exits** (3 disk, 4 memory, 5 heat, 6 orphaned CPU
  burner), so the background task notification wakes you. Act on it, then start the guard again.
  With `--keep-running` it prints alerts and carries on, for use under a Monitor.
- **Only one guard runs at a time.** A second one exits 2 and names the holder. Read the running
  guard's log instead: one JSON line per minute in `~/biorouter-runs/resource-guard/guard.jsonl`,
  every deletion as an `"event":"purge"` line with its size.
- **Check a running guard** by its heartbeat (the log's last line is under two minutes old) and by
  `ps -o pid,lstart,command -p <pid>`, not by its lock file alone.
- **A background task ends with its session.** Put the start command in your handoff.
- Add `--docker` to prune dangling images and old build cache at tier 2, and
  `--retire-idle-hours 12 --retire-root ~/biorouter-runs/<campaign>` to let it delete whole cargo
  targets idle that long under your own campaign folder (never a full clone's unless
  `--include-clones`). Do not make the repo's shared `.claude/worktrees` a retire root: its idle
  targets may belong to other sessions. List shared `CARGO_TARGET_DIR` folders in the protect
  file. Dry-run first whenever you widen a rule.

## Disk tiers

Free space on the data volume decides what the guard may delete with `--purge`, cheapest to
rebuild first. It stops once 120 GiB is free. Every number is a flag.

| Free | May delete | Cost to get it back |
|---|---|---|
| 80 GiB or more | nothing; `lane_room` in the log says how many more 40 GB lanes fit | none |
| under 80 (tier 1) | Homebrew, pip and npm download caches, skipped while brew, pip or npm runs; `target/*/deps/*.o` older than 6 h | a download; debug symbols of old binaries |
| under 60 (tier 2) | also: incremental dirs idle 60 min with no `-working` session; single-link test binaries older than 6 h that no running command names; with `--docker`, dangling images and build cache older than 7 days; with `--retire-idle-hours`, whole idle targets under a `--retire-root` that nothing uses | a slower next build; a relink; a full rebuild |
| under 40 (tier 3) | the same with shorter ages (`.o` 1 h, incremental 20 min, test binaries 3 h), then `ALERT disk` and exit 3 | as above |

A profile where cargo holds `.cargo-lock`, or where a build tool has its working directory, gets
only single-link `.o` files older than 6 h. rlib, rmeta, `.d`, fingerprints, `build/` and
top-level binaries are never deleted. How it decides, and the retired watchdog's defects that
shaped it: `reference/disk.md`.

**When the guard exits at tier 3:**

1. Freeze your own runaway build by pid: `kill -STOP <cargo pid>`, and `kill -CONT` after.
2. Find the growth: `du -sg ~/biorouter-runs/* | sort -n | tail`, then the worktree targets.
3. Retire your own finished lanes' targets, after checking no session uses the folder.
4. Park cold data on the approved external disk (below).
5. Ask the owner for anything else, and say what you already freed.

Report what the guard freed in your next update to the owner; no need to ask first.

## Never delete

Most of these look reclaimable and are not.

- Source, uncommitted work, handoffs, reports, receipts and evidence of any run.
- Git bundles and WIP patches: move them to the external disk instead.
- `ui/desktop/node_modules` (every worktree's is a symlink to it), `~/.cargo/registry`,
  `~/.cargo/git`, `~/.rustup`, `.hermit`, `.codegraph`.
- `ui/desktop/src/bin/` staged binaries, and the main checkout's `target/release` and
  `target/x86_64-apple-darwin` before a release (`just copy-binary` uses them).
- The Docker release volumes (`biorouter-linux-*`, `biorouter-windows-*`,
  `biorouter-crew-broker-target`), base images, BioRouterBench images, a running Crew lab.
- Local models under `~/.ollama` and the Playwright browser cache (its installer stalls here).
- Pinned binary folders a script points at, and folders whose `receipt.json` says their
  content was retained.
- The owner's main checkout and its working tree, other sessions' worktrees and their whole
  targets (the guard's tier pruning of caches inside them is fine), app data under
  `~/.config/biorouter` and `~/.local/share/biorouter`, credentials and keychains.
- Anything matched only by name: a search for the project name on the external disk finds the
  owner's manuscripts and cloud-drive safety copies.

## Cold storage on the external disk

The owner approved the external `WD_BLACK` volume (7.3 TB, exFAT) for large cold files when the
internal disk runs low. Other disks need the owner's word.

```bash
python3 -I $G/cold-store.py <folder> [<folder> ...] \
  --dest-root /Volumes/WD_BLACK/biorouter-backup/<campaign> --note "why" --remove-source
```

It writes one tar per folder (exFAT wastes a cluster per small file), checks the entry count,
hashes the archive by reading it back, appends to `MANIFEST.jsonl`, and removes the source only
when every check passed. It refuses folders written in the last 20 minutes, folders a process
uses, and folders holding credential files. Without `--remove-source` it keeps the source.
Never park what a live lane or a pinned path uses. Detail: `reference/disk.md`.

## Memory, CPU and heat

- **Memory:** pressure level 4 alerts at once; level 2 alerts with 200 or more swapouts per
  second for three samples. Then stop adding lanes and pause your own heaviest job by pid.
- **CPU:** the guard logs load, CPU idle and the top five users. Add lanes only while idle stays
  above about 10 %; below that they queue rather than finish sooner.
- **Heat:** `pmset -g therm` reporting a warning, or a speed limit under 100, alerts. None was
  recorded in any 2026-11 campaign, even at load 580.
- Readings, thresholds and how to act: `reference/processes.md`.

## Orphaned and runaway processes

- **Signature of a leak:** parent pid 1 plus sustained CPU on a shell, Python, node, vite, esbuild
  or cargo. The guard alerts after three samples over 50 %. It never kills anything.
- Stop a leak only when it is yours (your pid, or its working directory is your worktree),
  by pid. Bound any load generator with its own deadline; never rely on one `kill` at the end of a
  script.
- After e2e runs, look for vite and esbuild servers with parent pid 1 whose worktree is gone.
- **Not yours:** report it and ask, with the process name, its age and its CPU. On 2026-11-20 the
  owner approved quitting a 23-hour System Settings storage job that used 1.5 cores.
- Put this sentence in every subagent brief that can start a process: "NEVER kill processes by
  pattern; kill only pids you started, by pid."

## Stalled agents

```bash
python3 -I $G/stall-watch.py --session-dir ~/.claude/projects/<slug>/<session id> \
  --run <run id> [--run <run id>] --idle-min 25
```

Run it in the background beside long workflows. It prints one `STALL` line per running agent
whose transcript has been idle too long, with the last tool call and whether it still waits for
its result, then exits 5. Then: check the process, read the agent's uncommitted diff, stop the
workflow, and relaunch only the unfinished items. Never message a running workflow agent; that
forked a second live copy once. Detail: `reference/stalls-and-false-greens.md`.

## Results that lie

- `nohup ... &` inside a background call, a pipe into `tail`, and `cmd | tee log` each report the
  wrong exit code. Capture `rc=$?` right after the command and grep the work's own last line.
- A `pgrep -f` waiter matches its own command line; use `pgrep -f 'cargo tes[t]'`.
- A CI monitor must demand `success` per job; `cancelled` is neither success nor failure.
- zsh does not word-split `$var`; run multi-item loops under `bash -c` with an array.
- The full list, with the incidents: `reference/stalls-and-false-greens.md`.

## When to alert the owner

Send a PushNotification (one per incident, saying what you already tried) when:

- free disk stays under the tier 3 floor after a full pass and cold storage, and only items on
  the never-delete list or other people's work remain;
- you need to delete or move something outside the tier table, or something not yours;
- a process that is not yours burns CPU for a long time, or another session's daemon died;
- memory stays critical or a thermal warning persists after you paused your own work;
- a macOS privacy grant is needed (name the app path and the Settings pane; never change it
  yourself).

Everything in the tier table needs no message beforehand; report it in your next update.

## Files

| File | What it is |
|---|---|
| `scripts/resource-guard.py` | the guard: read-only sampling, optional tiered purge, alerts |
| `scripts/test-resource-guard.py` | negative control for every purge rule, on fake targets |
| `scripts/stall-watch.py` | read-only watch for workflow agents whose transcripts went quiet |
| `scripts/cold-store.py` | verified tar archives on the external disk, with a manifest |
| `reference/disk.md` | what fills the disk, what may go, Docker, cold storage, traps |
| `reference/processes.md` | memory, CPU, heat, orphans and safe killing |
| `reference/stalls-and-false-greens.md` | stalled agents and checks that report false success |

All four scripts use only the standard library and run under the system Python 3.9 with
`python3 -I`.

## Related documentation

- Biorouter `CLAUDE.md`: "Build & Release" (cargo profiles), "Releasing (cross-platform)" (the
  Docker caches and the `npm ci` rule), "Configuration" (where user data lives).
- `docs/desktop-ui/launching-the-dev-gui.md` for the dev GUI processes a cleanup must not break.
- The `campaign-orchestration` skill for usage caps, handoffs and workflow fan-out, and the
  `pr-ci-merge` skill for reading CI verdicts.
