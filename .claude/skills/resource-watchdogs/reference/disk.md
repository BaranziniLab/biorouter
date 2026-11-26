# Disk: what fills it, what may go, and how to measure

The detail behind the disk rules in `../SKILL.md`. Figures are measurements from the owner's Mac
(16 cores, 128 GiB, one internal data volume), dated so you can tell how old they are.

## Where the space goes

- **Rust `target/` dirs are nearly all of it.** A warm debug target is about 68 GB; a full
  `cargo test --workspace` in a worktree that lanes built per package grew one past 190 GB.
  Eleven worktrees once held 869 GiB of targets (2026-09-11).
- **Most of a bloated target is stale `.o` files.** macOS keeps debug info unpacked, so every
  codegen unit's object file stays in `deps/`, one set per rebuild. One 395 GB `deps/` was 350 GB
  of `.o` (2026-09-25). Deleting `.o` and old test executables took that target from 408 to 76 GB
  and kept the next builds incremental, because cargo judges freshness by the rlib, rmeta and
  executable outputs, not by the objects.
- **Campaign folders, not the repo, are often the surge.** Coordinator runs clone the repo per
  workstream round, and each clone builds its own target: 702 GiB under one campaign folder on
  2026-10-11. Look there first: `du -sg ~/biorouter-runs/* | sort -n | tail`.
- **Testers build targets in their sandboxes** to run covering tests, 70 to 80 GB each.
- **Docker Desktop's disk image** grows inside `~/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw`
  and gives space back to macOS a minute or so after a prune, not at once.
- **Local models** (Ollama and Llama Server downloads) were about 88 GiB until the owner had them
  deleted on 2026-11-20. Deleting or pulling models is the owner's call; re-pull before a
  local-model test.

## A cargo target, file by file

| Path under `target/<profile>/` | Safe to delete? |
|---|---|
| `deps/*.o` older than a few hours | yes; costs only debug symbols of binaries already linked |
| `deps/<name>-<hash>` (no extension, executable, one link) | yes when idle; cargo relinks it |
| `deps/<name>-<hash>` with two links | pointless: it shares an inode with a top-level binary |
| `incremental/<crate>/` with no `-working` session, idle | yes when no build holds the lock; next build of that crate is slower once |
| `deps/*.rlib`, `*.rmeta`, `*.d`, `.fingerprint/`, `build/` | no: deleting them forces a full rebuild |
| top-level binaries (`target/debug/biorouterd`) | no: dev GUIs and scripts run them |
| the whole target | only when idle for hours with nothing using it; costs a full rebuild |

Six hours is the default object age so running dev daemons keep their debug info; one hour is safe
for objects in an idle profile when the disk is critical.

## How the guard decides a build is using a target

1. **The cargo lock.** Cargo holds an exclusive `flock` on `target/<profile>/.cargo-lock` while it
   builds. The guard takes the same lock without blocking and holds it while it deletes, so a build
   that starts meanwhile waits instead of breaking.
2. **Working directories.** `lsof -a -d cwd` for cargo, rustc, clippy, rustdoc, ld, cc and clang.
   A build tool sitting in the checkout marks it busy. Note that lsof exits 1 when one of several
   `-c` names matched nothing; with `-w`, exit 1 and an empty stderr is still a good answer.
3. **Command lines.** A file named on any running command line (a test binary being run) stays.
4. **Open files, for whole-target retirement only.** A full `lsof -nP -Fn` (under a second for
   80,000 open files here); any open path under the target keeps it.
5. **Entry mtimes, not directory mtimes, for idle time.** The guard's own deletions bump a
   directory's mtime; reading that as activity kept pruned targets looking busy.

If any of 2 to 4 fails, the guard skips every step that needs it. Busy profiles get only
single-link `.o` files older than six hours.

Whole-target retirement also needs every profile lock, no full clone (a `.git` directory beside
the target) unless `--include-clones`, and a target under a `--retire-root`. The owner's
2026-11-17 approval covered idle targets with no process using them, but whose target it is
still matters: the desktop app reuses worktree folders for new sessions, so a target in the
repo's shared worktrees folder can belong to a live session that is merely quiet. A mistaken
retirement costs a rebuild, never source. A shared `CARGO_TARGET_DIR` outside any checkout looks
like any other idle target; list it in the protect file while a lane still uses it.

## Why the 2026-11 deleting watchdog was retired

A review on 2026-11-19 stopped it and left a read-only logger in its place. Its defects are the
reasons for rules in `../SKILL.md`:

- it protected listed paths only from whole-target deletion and still pruned `.o` and incremental
  files under them;
- an `lsof` or `ps` that failed or timed out read as "nothing busy" (fail open);
- its list of active worktrees went stale and missed the release worktree and a shared target;
- a second cleanup ran beside it, the two deleted the same files, and neither could then prove
  what it had removed;
- it logged logical sizes as freed space ("removed 2.8 GB, freed 0.0").

`../scripts/resource-guard.py` fixes each one, and `../scripts/test-resource-guard.py` fails if
any fix is reverted.

## Measure reclaim with df

- `du` counts every APFS clone and hard link in full. Five campaign targets measured 689 GiB by
  `du` and freed 151 GiB (2026-10-11); a 14 GiB Codex worktree target freed 0.6 GiB.
- Read free space before and after: `df -g ~ | awk 'NR==2{print $4}'`.
- The guard logs `single_link_gib` (blocks only that file holds) beside `logical_gib`, plus the
  `df` reading before and after each pass. Quote the `df` delta.
- Deleting hundreds of GB of small files takes many minutes and `df` barely moves at first. Run it
  in the background and watch `df`; it has not hung.

## Docker

- **Never prune these volumes.** `docker system df` calls them reclaimable; they are the release
  and cross-compile caches named in `scripts/release.sh`, `scripts/cross-env.sh`, the Justfile and
  `scripts/build-crew-broker-linux.sh`: `biorouter-linux-bullseye-cache`, `biorouter-linux-npm-cache`,
  `biorouter-linux-release-target`, `biorouter-windows-cache`, `biorouter-windows-release-target`,
  `biorouter-crew-broker-target`. Losing them costs about 40 minutes on the next release.
- **Never remove** base images (`rust:1.92-bullseye`, `node:24-bookworm`, `rockylinux:9`,
  `debian:bookworm-slim`), BioRouterBench images (`biorouterbench-*`, `bb*-local`, pinned for
  reproducibility), a running Crew lab container, or small identity volumes.
- **Safe without asking:** `docker image prune -f` (dangling only) and
  `docker builder prune -a -f --filter until=168h` (build cache older than 7 days). The guard runs
  only these, with `--docker`. A builder prune freed 15.8 GB on 2026-10-25.
- **By hand, under the 2026-11-17 approval:** your own campaign's test containers stopped for more
  than a day, its unused test images older than three days, and its ad-hoc test volumes, matched
  against the names your campaign gave them. Anything you cannot prove is yours: ask.
- **Docker Desktop wedges after a disk-full moment** and will not start while old backends linger.
  The reliable restart is the one pattern kill this skill allows, because the path names only
  Docker Desktop's own processes; run it only once you know no other session's containers run:
  `pkill -f "Docker.app/Contents/MacOS/com.docker"; sleep 5; open -a Docker`. Check health with a
  mounted container, not a bare one: `docker run --rm -v "$PWD:/w" -w /w alpine ls /w`.
- A hung `docker` CLI can ignore signals; give every Docker call a timeout.

## Keep disk use down while building

- **One target per worktree.** Never point two worktrees of the same package at one
  `CARGO_TARGET_DIR`: their unit hashes are identical, the last build wins, and one tree silently
  tests the other's code.
- **Lean builds for parallel lanes:** `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0
  CARGO_PROFILE_TEST_DEBUG=0` (environment only) kept a worktree that built lib tests, the daemon,
  the CLI and clippy at 6.5 GB. Backtraces lose line numbers.
- **APFS clone a sibling target** when the disk is tight: `cp -Rc <idle sibling>/target <mine>/target`
  costs almost no space. It saves disk, not build time: cargo still rebuilt the whole dependency
  graph in measured runs. Never delete a sibling's target to make room for yours.
- **The first build after a purge is cold.** Tell the next agent, or it will read a slow build as
  a stuck one.
- `df -h ~` before adding a lane; a new lane needs 30 to 70 GB.

## Traps

- **`rm -rf "$d"` on a name that starts with `-`** reads it as flags. Claude scratchpad folders are
  named `-Users-...`. Use `rm -rf -- "$d"`.
- **Worktree folder names say nothing about who uses them.** The desktop app reuses old worktree
  folders for new sessions. Before deleting anything in one, check the sessions list (each
  session's `cwd` and branch), `git -C <wt> branch --show-current` and running processes.
- **Check a branch is pushed against `git ls-remote`, not a rev range.** `git log origin/$br..HEAD`
  is empty when `origin/$br` does not exist, which reads as "nothing unpushed". This matters when
  you remove a worktree, not just its target.
- **Receipts can forbid deletion.** Leave folders whose `receipt.json` asserts content was
  retained (the `legacy-*-retirement*` folders): deleting inside them falsifies the record.
- **Scratchpads under `/private/tmp/claude-<uid>/`** whose worktree no longer exists are safe to
  remove; keep your own session's and any live worktree's.
- **A name match is not a reason.** A search for the project name on the external disk matches
  the owner's manuscripts and Box or Dropbox safety copies.

## Cold storage on the external disk

The owner approved parking large cold files on the external `WD_BLACK` volume (7.3 TB, exFAT) when
the internal disk runs low (2026-11-10). Other external disks need the owner's word.

- **Write one tar per folder, never a folder copy.** exFAT allocates whole clusters (128 KB on the
  owner's other external disk), so a tree of small files can need several times its size. tar also
  keeps modes and symlinks that exFAT does not.
- **Verify before removing the source:** the archive's entry count must equal the source's, and
  the sha256 is computed by reading the archive back from the external disk. Record each archive in
  `MANIFEST.jsonl` beside it. `../scripts/cold-store.py` does all of this.
- **Do not trust `df` on the exFAT disk;** it has reported the same free space for a copy that
  succeeded and one that hit "no space left". A failed write is the real signal.
- **Never move** what a live lane or a pinned path uses (a released-binaries folder an install
  matrix points at), and never carry credential files (`secrets.yaml`, `.env`, keys) to the
  external disk. Delete those copies first, or ask.
- **Git bundles and WIP patches move here instead of being deleted.** A bundle can be the only copy
  of a force-deleted branch.
- macOS ships openrsync, which rejects rsync 3 flags such as `--info=progress2`; another reason to
  use tar.
- Moving many small files is slow (minutes per GB); run it in the background.

## Related documentation

- `../SKILL.md`, the rules and the tier table
- `processes.md` in this folder
- Biorouter `CLAUDE.md`, "Build & Release" (cargo profiles) and "Releasing (cross-platform)" (the
  Docker caches and the `node_modules` rule)
