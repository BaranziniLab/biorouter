#!/usr/bin/env python3
"""Resource guard for long agent campaigns on the owner's Mac.

Read-only by default. Every --interval seconds it appends one JSON line to --log with free
disk, memory pressure, current paging, swap occupancy, CPU load and idle time, thermal state,
open files, the top CPU users and any orphaned CPU burners.

With --purge it also frees documented rebuildable caches, cheapest first, once free disk falls
below a tier (see ../SKILL.md, "Disk tiers"), and logs every deletion with its size. It never
deletes anything else, never kills or stops a process, and skips a cleanup step whenever it
cannot tell whether a build is using the files (fail closed).

Alerts print one line that starts with ALERT. Unless --keep-running is given, the guard then
exits with the alert's code, so a background task notification wakes the orchestrator.

Exit codes: 0 stopped, or --once with no alert; 1 guard error; 2 usage error or another purging
guard already holds the lock; 3 disk; 4 memory; 5 thermal; 6 orphaned CPU burner.

Run it with `python3 -I` so nothing in the current directory is imported. Standard library
only; works with the system Python 3.9. Tested on macOS; on Linux the macOS-only probes
(memory pressure, paging, thermal, CPU idle) are recorded as null.
"""
import argparse
import fcntl
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time

GIB = 1024 ** 3
EXIT_ERROR, EXIT_USAGE, EXIT_DISK, EXIT_MEMORY, EXIT_THERMAL, EXIT_ORPHAN = 1, 2, 3, 4, 5, 6

# Download caches whose contents are fetched again on demand. Each is skipped while a process
# whose command line matches its pattern runs, because that process may be writing the cache.
DOWNLOAD_CACHES = {
    "~/Library/Caches/Homebrew/downloads": r"\bbrew\b|Homebrew/",
    "~/Library/Caches/pip": r"\bpip3?\b",
    "~/.npm/_cacache": r"\bnpm\b|\bnpx\b|npm-cli",
}

# Path fragments the guard refuses to delete under, whatever a rule says (defence in depth).
NEVER_PARTS = (
    "/node_modules", "/.cargo/registry", "/.cargo/git", "/.rustup", "/.hermit", "/.codegraph",
    "/ui/desktop/src/bin", "/.git/", "/.ollama", "/ms-playwright", "/Library/Keychains",
    "/.config/biorouter", "/.local/share/biorouter", "/.local/state/biorouter", "/.ssh",
)

# Directories the target search never descends into.
SKIP_DIRS = {
    "node_modules", ".git", ".hermit", ".codegraph", ".venv", "venv", "__pycache__",
    "site-packages", "crates", "docs", "landing", "vendor",
}

# lsof -c matches a command-name prefix: "cargo" covers cargo-clippy and cargo-nextest,
# "rust" covers rustc, rustdoc, rust-lld and rust-analyzer.
BUILD_COMMANDS = ("cargo", "rust", "clippy", "ld", "cc", "clang")

# An orphan (parent pid 1) with one of these names and sustained CPU is a leaked loop or
# harness. System daemons and apps also have parent pid 1, so their paths are excluded.
# The system Python runs as ".../Python.app/Contents/MacOS/Python", hence the case-insensitive match.
ORPHAN_NAME = re.compile(
    r"^(zsh|bash|sh|dash|fish|python[0-9.]*|node|deno|bun|vite|esbuild|cargo[-a-z]*|rustc|perl|ruby|yes)$",
    re.I,
)
SYSTEM_PREFIXES = ("/System/", "/usr/libexec/", "/usr/sbin/", "/sbin/")

STOP = False


def now_iso():
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())


def expand(path):
    return os.path.realpath(os.path.expanduser(path))


def run(cmd, timeout=30):
    """Return (returncode, stdout, stderr); returncode None when the command could not run."""
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.SubprocessError) as exc:
        return None, "", str(exc)
    return r.returncode, r.stdout, r.stderr


def output(cmd, timeout=30):
    """Stdout of a command that succeeded, else None."""
    rc, out, _ = run(cmd, timeout)
    return out if rc == 0 else None


def lsof(args, timeout=120):
    """Stdout of lsof, or None when it failed. lsof exits 1 when a selection matched nothing;
    with -w that is the only reason it exits 1 with an empty stderr, so that counts as success."""
    rc, out, err = run(["lsof", "-w"] + args, timeout)
    if rc == 0 or (rc == 1 and not err.strip()):
        return out
    return None


def free_gib(volume):
    st = os.statvfs(volume)
    return st.f_bavail * st.f_frsize / GIB


def inside(path, parent):
    return path == parent or path.startswith(parent.rstrip("/") + "/")


def overlaps(path, parent):
    return inside(path, parent) or inside(parent, path)


# ---------------------------------------------------------------------------- sampling


def iostat_idle():
    out = output(["iostat", "-c", "2", "-w", "1", "-n", "0"], timeout=15)
    if not out:
        return None
    lines = [l.split() for l in out.splitlines() if l.strip()]
    header = next((l for l in lines if "id" in l and "us" in l), None)
    data = [l for l in lines if l and re.match(r"^\d", l[0])]
    if not header or not data:
        return None
    try:
        return float(data[-1][header.index("id")])
    except (ValueError, IndexError):
        return None


def vm_counters():
    out = output(["vm_stat"])
    if not out:
        return None
    page = re.search(r"page size of (\d+) bytes", out)
    counters = {"page_bytes": int(page.group(1)) if page else 4096}
    for key in ("Swapins", "Swapouts", "Pageouts"):
        m = re.search(rf"^{key}:\s+(\d+)", out, re.M)
        if m:
            counters[key.lower()] = int(m.group(1))
    return counters


def swap_used_gib():
    out = output(["sysctl", "-n", "vm.swapusage"])
    m = re.search(r"used = ([\d.]+)M", out or "")
    return round(float(m.group(1)) / 1024, 1) if m else None


def memory_pressure():
    level = output(["sysctl", "-n", "kern.memorystatus_vm_pressure_level"])
    out = output(["memory_pressure", "-Q"])
    m = re.search(r"free percentage:\s*(\d+)%", out or "")
    try:
        level = int(level.strip()) if level else None
    except ValueError:
        level = None
    return level, (int(m.group(1)) if m else None)


def thermal():
    """A warning line, "" when pmset reports none, None when pmset is unavailable."""
    out = output(["pmset", "-g", "therm"])
    if out is None:
        return None
    for line in out.splitlines():
        text = line.strip()
        limit = re.search(r"CPU_Speed_Limit\s*=\s*(\d+)", text)
        if limit and int(limit.group(1)) < 100:
            return text
        if re.search(r"(thermal|performance) warning level", text, re.I) and not text.lower().startswith("note: no"):
            return text
    return ""


def open_files_pct():
    used = output(["sysctl", "-n", "kern.num_files"])
    limit = output(["sysctl", "-n", "kern.maxfiles"])
    try:
        return round(100 * int(used) / int(limit), 1)
    except (TypeError, ValueError, ZeroDivisionError):
        return None


def processes():
    """[(pid, ppid, pcpu, etime, comm)] or None when ps failed."""
    out = output(["ps", "-axww", "-o", "pid=,ppid=,pcpu=,etime=,comm="])
    if out is None:
        return None
    rows = []
    for line in out.splitlines():
        parts = line.split(None, 4)
        if len(parts) < 5:
            continue
        try:
            rows.append((int(parts[0]), int(parts[1]), float(parts[2]), parts[3], parts[4].strip()))
        except ValueError:
            continue
    return rows


def command_lines():
    """Every running command line joined into one string, or None when ps failed."""
    return output(["ps", "-axww", "-o", "command="])


# ---------------------------------------------------------------------------- cargo targets


def profile_dirs(target):
    """Cargo profile dirs (holding .cargo-lock) at target/<profile> or target/<triple>/<profile>."""
    found = []
    try:
        level1 = [os.path.join(target, d) for d in sorted(os.listdir(target))]
    except OSError:
        return found
    for d in level1:
        if os.path.islink(d) or not os.path.isdir(d):
            continue
        if os.path.exists(os.path.join(d, ".cargo-lock")):
            found.append(d)
            continue
        try:
            for e in sorted(os.listdir(d)):
                p = os.path.join(d, e)
                if not os.path.islink(p) and os.path.isdir(p) and os.path.exists(os.path.join(p, ".cargo-lock")):
                    found.append(p)
        except OSError:
            pass
    return found


def find_targets(roots, max_depth):
    targets = []
    for base in roots:
        if not os.path.isdir(base):
            continue
        for root, dirs, files in os.walk(base):
            depth = root[len(base):].count(os.sep)
            if ("CACHEDIR.TAG" in files or ".rustc_info.json" in files) and profile_dirs(root):
                targets.append(root)
                dirs[:] = []
                continue
            if "Cargo.toml" in files and "crates" in dirs:
                # A workspace root: only its target dir and its agent worktrees hold build output.
                dirs[:] = [d for d in dirs if d in ("target", ".claude")]
            elif os.path.basename(root) == ".claude":
                dirs[:] = [d for d in dirs if d == "worktrees"]
            else:
                dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
            if depth >= max_depth:
                dirs[:] = []
    return targets


def owner_dir(target):
    """The checkout a target belongs to, when it sits directly inside one."""
    parent = os.path.dirname(target)
    if any(os.path.exists(os.path.join(parent, n)) for n in ("Cargo.toml", ".git")):
        return parent
    return None


def is_full_clone(target):
    parent = owner_dir(target)
    return bool(parent) and os.path.isdir(os.path.join(parent, ".git"))


def try_lock(profile):
    try:
        fd = os.open(os.path.join(profile, ".cargo-lock"), os.O_RDWR)
    except OSError:
        return None
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return fd
    except OSError:
        os.close(fd)
        return None


def unlock(fd):
    if fd is not None:
        try:
            fcntl.flock(fd, fcntl.LOCK_UN)
        finally:
            os.close(fd)


def newest_output(target):
    """Newest mtime among the entries a build writes in deps/, .fingerprint/ and build/.

    Entry mtimes, not directory mtimes: the guard's own deletions bump a directory's mtime, and
    reading that as build activity would keep a pruned target looking busy for another day."""
    newest = 0.0
    for prof in profile_dirs(target):
        for sub in ("deps", ".fingerprint", "build"):
            try:
                with os.scandir(os.path.join(prof, sub)) as it:
                    for e in it:
                        try:
                            newest = max(newest, e.stat(follow_symlinks=False).st_mtime)
                        except OSError:
                            pass
            except OSError:
                pass
    return newest or os.stat(target).st_mtime


def tree_size(path):
    """(logical bytes, bytes only this path holds: single-link files' allocated blocks)."""
    logical = single = 0
    for root, dirs, files in os.walk(path):
        for name in files + dirs:
            try:
                st = os.lstat(os.path.join(root, name))
            except OSError:
                continue
            if name in files:
                logical += st.st_size
                if st.st_nlink == 1:
                    single += st.st_blocks * 512
    return logical, single


# ---------------------------------------------------------------------------- the guard


class Guard:
    def __init__(self, args):
        self.a = args
        self.volume = expand(args.volume)
        self.log_path = expand(args.log)
        self.roots = [expand(r) for r in args.root]
        self.retire_roots = [expand(r) for r in args.retire_root]
        self.prev_vm = None
        self.prev_t = None
        self.paging_streak = 0
        self.orphan_streak = {}
        self.last_purge = (0.0, 0)  # (time, tier)
        self.alerted = {}
        self.errors = 0
        os.makedirs(os.path.dirname(self.log_path), exist_ok=True)

    # -- logging

    def log(self, record):
        record.setdefault("t", now_iso())
        try:
            if os.path.getsize(self.log_path) > self.a.max_log_mb * 1024 * 1024:
                os.replace(self.log_path, self.log_path + ".1")
        except OSError:
            pass
        with open(self.log_path, "a") as f:
            f.write(json.dumps(record, separators=(",", ":")) + "\n")

    def protected(self):
        paths = [expand(p) for p in self.a.protect]
        if self.a.protect_file:
            try:
                with open(expand(self.a.protect_file)) as f:
                    for line in f:
                        line = line.strip()
                        if line and not line.startswith("#"):
                            paths.append(expand(line))
            except OSError as exc:
                # A protect list that cannot be read protects nothing: stop deleting instead.
                return None, str(exc)
        return paths, None

    # -- sampling

    def sample(self):
        s = {"event": "sample", "volume": self.a.volume}
        s["free_gib"] = round(free_gib(self.volume), 1)
        load = os.getloadavg()
        s["load"] = [round(x, 1) for x in load]
        s["ncpu"] = os.cpu_count()
        s["cpu_idle_pct"] = iostat_idle()
        s["mem_pressure_level"], s["mem_free_pct"] = memory_pressure()
        s["swap_used_gib"] = swap_used_gib()
        vm, t = vm_counters(), time.time()
        s["swapouts_per_s"] = s["swapins_per_s"] = None
        if vm and self.prev_vm and t > self.prev_t:
            dt = t - self.prev_t
            for key in ("swapouts", "swapins"):
                if key in vm and key in self.prev_vm:
                    s[f"{key}_per_s"] = round(max(0, vm[key] - self.prev_vm[key]) / dt, 1)
        self.prev_vm, self.prev_t = vm, t
        s["thermal"] = thermal()
        s["open_files_pct"] = open_files_pct()
        rows = processes()
        if rows is not None:
            s["procs"] = len(rows)
            names = [os.path.basename(r[4]) for r in rows]
            s["cargo"] = sum(1 for n in names if n == "cargo")
            s["rustc"] = sum(1 for n in names if n == "rustc")
            top = sorted(rows, key=lambda r: -r[2])[:5]
            s["top"] = [[r[0], r[2], os.path.basename(r[4])[:40]] for r in top]
            orphans = [r for r in rows
                       if r[1] == 1 and r[2] >= self.a.orphan_cpu
                       and not r[4].startswith(SYSTEM_PREFIXES)
                       and ORPHAN_NAME.match(os.path.basename(r[4]))]
            seen = {r[0] for r in orphans}
            self.orphan_streak = {p: n for p, n in self.orphan_streak.items() if p in seen}
            for r in orphans:
                self.orphan_streak[r[0]] = self.orphan_streak.get(r[0], 0) + 1
            s["orphans"] = [[r[0], r[2], r[3], os.path.basename(r[4])] for r in orphans]
        level, outs = s["mem_pressure_level"], s["swapouts_per_s"]
        if level is not None and level >= 2 and outs is not None and outs >= self.a.swapout_rate:
            self.paging_streak += 1
        else:
            self.paging_streak = 0
        s["tier"] = self.tier(s["free_gib"])
        s["lane_room"] = max(0, int((s["free_gib"] - self.a.t1) // self.a.lane_gib))
        return s

    def tier(self, free):
        if free >= self.a.t1:
            return 0
        if free >= self.a.t2:
            return 1
        if free >= self.a.t3:
            return 2
        return 3

    # -- deletion primitives

    def refuse(self, path, protected):
        real = os.path.realpath(path)
        if any(part in real + "/" for part in NEVER_PARTS):
            return "never-delete path"
        if any(overlaps(real, p) for p in protected):
            return "protected"
        return None

    def remove_tree(self, path, rule, protected, extra=None):
        why = self.refuse(path, protected)
        if why or os.path.islink(path):
            self.log({"event": "skip", "rule": rule, "path": path, "why": why or "symlink"})
            return 0
        if os.path.isdir(path):
            logical, single = tree_size(path)
        else:
            try:
                st = os.lstat(path)
            except OSError:
                return 0
            logical, single = st.st_size, (st.st_blocks * 512 if st.st_nlink == 1 else 0)
        if not self.a.dry_run:
            if os.path.isdir(path):
                shutil.rmtree(path, ignore_errors=True)
            else:
                try:
                    os.unlink(path)
                except OSError:
                    pass
        rec = {"event": "purge", "rule": rule, "path": path, "logical_gib": round(logical / GIB, 2),
               "single_link_gib": round(single / GIB, 2), "dry_run": self.a.dry_run}
        rec.update(extra or {})
        self.log(rec)
        return single

    def remove_files(self, paths, rule, where, protected):
        logical = single = n = 0
        for p in paths:
            if self.refuse(p, protected):
                continue
            try:
                st = os.lstat(p)
                if not self.a.dry_run:
                    os.unlink(p)
            except OSError:
                continue
            n += 1
            logical += st.st_size
            if st.st_nlink == 1:
                single += st.st_blocks * 512
        if n:
            self.log({"event": "purge", "rule": rule, "path": where, "files": n,
                      "logical_gib": round(logical / GIB, 2), "single_link_gib": round(single / GIB, 2),
                      "dry_run": self.a.dry_run})
        return single

    # -- purge steps, cheapest to rebuild first

    def purge_download_caches(self, ctx, protected):
        if self.a.skip_download_caches:
            return
        for raw, pattern in DOWNLOAD_CACHES.items():
            cache = expand(raw)
            if not os.path.isdir(cache):
                continue
            if ctx["commands"] is None or re.search(pattern, ctx["commands"]):
                self.log({"event": "skip", "rule": "download-cache", "path": cache,
                          "why": "a tool that writes this cache may be running"})
                continue
            for name in sorted(os.listdir(cache)):
                if self.goal_met():
                    return
                self.remove_tree(os.path.join(cache, name), "download-cache", protected)

    def deps_files(self, profile, kind, age_h, single_link_only, commands):
        deps = os.path.join(profile, "deps")
        if os.path.islink(deps) or not os.path.isdir(deps):
            return []
        cutoff = time.time() - age_h * 3600
        picked = []
        with os.scandir(deps) as it:
            for e in it:
                try:
                    if e.is_symlink() or not e.is_file(follow_symlinks=False):
                        continue
                    st = e.stat(follow_symlinks=False)
                except OSError:
                    continue
                if st.st_mtime > cutoff or (single_link_only and st.st_nlink != 1):
                    continue
                if kind == "object" and not e.name.endswith(".o"):
                    continue
                if kind == "test-binary":
                    # Extension-less executables in deps/ are test and bin copies. A copy that
                    # shares its inode with a top-level binary frees nothing, so skip it.
                    if "." in e.name or not (st.st_mode & 0o100) or st.st_nlink != 1:
                        continue
                    if commands is None or e.path in commands:
                        continue
                picked.append(e.path)
        return picked

    def idle_incremental(self, profile, idle_min):
        inc = os.path.join(profile, "incremental")
        if os.path.islink(inc) or not os.path.isdir(inc):
            return []
        cutoff = time.time() - idle_min * 60
        picked = []
        for name in sorted(os.listdir(inc)):
            crate = os.path.join(inc, name)
            if os.path.islink(crate) or not os.path.isdir(crate):
                continue
            try:
                children = os.listdir(crate)
                newest = max([os.stat(crate).st_mtime] +
                             [os.lstat(os.path.join(crate, c)).st_mtime for c in children])
            except OSError:
                continue
            if newest > cutoff or any(c.endswith("-working") for c in children):
                continue
            picked.append(crate)
        return picked

    def purge_targets(self, tier, ctx, protected):
        deep = tier >= 3
        for target in find_targets(self.roots, self.a.max_depth):
            if self.goal_met():
                return
            if any(inside(target, p) for p in protected):
                continue
            home = owner_dir(target)
            busy_cwd = ctx["cwds"] is None or (home is not None and any(inside(c, home) for c in ctx["cwds"]))
            for prof in profile_dirs(target):
                if self.goal_met():
                    return
                if any(overlaps(prof, p) for p in protected):
                    continue
                fd = try_lock(prof)
                idle = fd is not None and not busy_cwd
                try:
                    if idle:
                        age = self.a.low_obj_age_hours if deep else self.a.obj_age_hours
                        self.remove_files(self.deps_files(prof, "object", age, False, None),
                                          "old-objects", prof, protected)
                        if tier >= 2:
                            inc_idle = self.a.low_incremental_idle_min if deep else self.a.incremental_idle_min
                            for crate in self.idle_incremental(prof, inc_idle):
                                self.remove_tree(crate, "idle-incremental", protected)
                            exe_age = self.a.low_exe_age_hours if deep else self.a.exe_age_hours
                            self.remove_files(self.deps_files(prof, "test-binary", exe_age, True, ctx["commands"]),
                                              "old-test-binaries", prof, protected)
                    else:
                        # A build may be running: only single-link objects old enough that no
                        # current link step reads them.
                        self.remove_files(self.deps_files(prof, "object", self.a.obj_age_hours, True, None),
                                          "old-objects-busy", prof, protected)
                finally:
                    unlock(fd)

    def retire_idle_targets(self, ctx, protected):
        if not self.a.retire_idle_hours:
            return
        if ctx["cwds"] is None or ctx["commands"] is None:
            self.log({"event": "skip", "rule": "retire-target", "why": "process state unknown"})
            return
        open_out = lsof(["-nP", "-Fn"], timeout=180)
        if open_out is None:
            self.log({"event": "skip", "rule": "retire-target", "why": "lsof failed"})
            return
        open_paths = [l[1:] for l in open_out.splitlines() if l.startswith("n/")]
        for target in find_targets(self.retire_roots, self.a.max_depth):
            if self.goal_met():
                return
            if any(overlaps(target, p) for p in protected):
                continue
            if is_full_clone(target) and not self.a.include_clones:
                continue
            home = owner_dir(target)
            if home and any(inside(c, home) for c in ctx["cwds"]):
                continue
            idle_h = (time.time() - newest_output(target)) / 3600
            if idle_h < self.a.retire_idle_hours or target in ctx["commands"]:
                continue
            if any(inside(p, target) for p in open_paths):
                continue
            fds = [try_lock(p) for p in profile_dirs(target)]
            try:
                if all(fd is not None for fd in fds):
                    self.remove_tree(target, "retire-idle-target", protected,
                                     {"idle_hours": round(idle_h, 1)})
            finally:
                for fd in fds:
                    unlock(fd)

    def purge_docker(self):
        if not self.a.docker:
            return
        rc, _, _ = run(["docker", "info", "--format", "{{.ServerVersion}}"], timeout=20)
        if rc != 0:
            self.log({"event": "skip", "rule": "docker", "why": "docker not answering"})
            return
        steps = (("docker-dangling-images", ["docker", "image", "prune", "-f"], 300),
                 ("docker-build-cache-7d", ["docker", "builder", "prune", "-a", "-f", "--filter", "until=168h"], 600))
        for rule, cmd, timeout in steps:
            if self.a.dry_run:
                self.log({"event": "purge", "rule": rule, "cmd": " ".join(cmd), "dry_run": True})
                continue
            rc, out, err = run(cmd, timeout)
            last = (out.strip().splitlines() or [err.strip() or "no output"])[-1]
            self.log({"event": "purge", "rule": rule, "rc": rc, "result": last[:200], "dry_run": False})

    def goal_met(self):
        return not self.a.dry_run and free_gib(self.volume) >= self.a.goal_gib

    def purge(self, tier):
        t, last_tier = self.last_purge
        if time.time() - t < self.a.cooldown_min * 60 and tier <= last_tier:
            return
        self.last_purge = (time.time(), tier)
        protected, err = self.protected()
        if protected is None:
            self.log({"event": "skip", "rule": "all", "why": f"protect file unreadable: {err}"})
            return
        out = lsof(["-a", "-d", "cwd", "-Fn"] + sum((["-c", c] for c in BUILD_COMMANDS), []), timeout=60)
        cwds = None if out is None else [l[1:] for l in out.splitlines() if l.startswith("n/")]
        ctx = {"cwds": cwds, "commands": command_lines()}
        if cwds is None:
            self.log({"event": "skip", "rule": "idle-only steps", "why": "lsof failed; treating every target as busy"})
        before = free_gib(self.volume)
        self.purge_download_caches(ctx, protected)
        self.purge_targets(tier, ctx, protected)
        if tier >= 2 and not self.goal_met():
            self.purge_docker()
            self.retire_idle_targets(ctx, protected)
        self.log({"event": "purge_pass", "tier": tier, "free_before_gib": round(before, 1),
                  "free_after_gib": round(free_gib(self.volume), 1), "dry_run": self.a.dry_run})

    # -- alerts

    def alerts(self, s):
        found = []
        floor = self.a.alert_gib if self.a.alert_gib is not None else (self.a.t3 if self.a.purge else self.a.t2)
        free = s.get("free_after_gib", s["free_gib"])
        if free < floor:
            what = "after a purge pass" if self.a.purge else "(guard is read-only)"
            found.append(("disk", EXIT_DISK, f"{free:.1f} GiB free on {self.a.volume} {what}, floor {floor:g} GiB"))
        if s["mem_pressure_level"] is not None and s["mem_pressure_level"] >= 4:
            found.append(("memory", EXIT_MEMORY, f"memory pressure critical (level {s['mem_pressure_level']}, "
                                                  f"{s['mem_free_pct']}% free)"))
        elif self.paging_streak >= self.a.streak:
            found.append(("memory", EXIT_MEMORY, f"memory pressure level {s['mem_pressure_level']} with "
                                                  f"{s['swapouts_per_s']} swapouts/s for {self.paging_streak} samples"))
        if s["thermal"]:
            found.append(("thermal", EXIT_THERMAL, f"thermal: {s['thermal']}"))
        if not self.a.no_orphan_alert:
            hot = [o for o in s.get("orphans", []) if self.orphan_streak.get(o[0], 0) >= self.a.streak]
            if hot:
                desc = ", ".join(f"pid {o[0]} {o[3]} {o[1]:.0f}% cpu up {o[2]}" for o in hot[:5])
                found.append(("orphan", EXIT_ORPHAN, f"orphaned CPU burners (parent pid 1): {desc}"))
        return found

    def report(self, alerts):
        fresh = []
        for kind, code, msg in alerts:
            if self.a.keep_running and time.time() - self.alerted.get(kind, 0) < 1800:
                continue
            self.alerted[kind] = time.time()
            fresh.append((kind, code, msg))
            print(f"ALERT {kind}: {msg}; log {self.log_path}", flush=True)
            self.log({"event": "alert", "kind": kind, "message": msg})
        return fresh

    # -- main loop

    def cycle(self):
        s = self.sample()
        if self.a.purge and s["tier"] > 0 and s["free_gib"] < self.a.goal_gib:
            self.purge(s["tier"])
            s["free_after_gib"] = round(free_gib(self.volume), 1)
        self.log(s)
        return s, self.report(self.alerts(s))


def summary(s, guard):
    a = guard.a
    lines = [
        f"disk: {s['free_gib']} GiB free on {a.volume} (tier {s['tier']}; tiers {a.t1:g}/{a.t2:g}/{a.t3:g} GiB); "
        f"room for {s['lane_room']} more build lane(s) at {a.lane_gib:g} GiB each",
        f"memory: pressure level {s['mem_pressure_level']} (1 normal, 2 warn, 4 critical), {s['mem_free_pct']}% free; "
        f"swap occupied {s['swap_used_gib']} GiB (history, not paging)",
        f"cpu: load {s['load']} on {s['ncpu']} cores, idle {s['cpu_idle_pct']}%; cargo {s.get('cargo')}, rustc {s.get('rustc')}",
        f"thermal: {s['thermal'] or 'no warning' if s['thermal'] is not None else 'unknown'}; open files {s['open_files_pct']}% of limit",
        f"top cpu: {s.get('top')}",
        f"orphans over {a.orphan_cpu:g}% cpu: {s.get('orphans') or 'none'}",
    ]
    if "free_after_gib" in s:
        lines.insert(1, f"after purge pass: {s['free_after_gib']} GiB free{' (dry run: nothing deleted)' if a.dry_run else ''}")
    return "\n".join(lines)


def take_lock(path):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    fd = os.open(path, os.O_RDWR | os.O_CREAT, 0o600)
    try:
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        os.close(fd)
        return None
    with open(path + ".json", "w") as f:
        json.dump({"pid": os.getpid(), "argv": sys.argv, "started": now_iso()}, f)
    return fd


def parse_args(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0],
                                formatter_class=argparse.ArgumentDefaultsHelpFormatter)
    p.add_argument("--purge", action="store_true", help="delete documented rebuildable caches by tier")
    p.add_argument("--dry-run", action="store_true", help="with --purge: log what would go, delete nothing")
    p.add_argument("--once", action="store_true", help="one sample (and purge pass), print a summary, exit")
    p.add_argument("--keep-running", action="store_true", help="print alerts and keep going instead of exiting")
    p.add_argument("--interval", type=float, default=60, help="seconds between samples")
    p.add_argument("--volume", default="~", help="measure free space on the volume holding this path")
    p.add_argument("--log", default="~/biorouter-runs/resource-guard/guard.jsonl", help="JSONL log")
    p.add_argument("--lock", default="~/biorouter-runs/.resource-guard.lock",
                   help="machine-wide lock: one purging guard at a time")
    p.add_argument("--max-log-mb", type=float, default=20, help="rotate the log to .1 past this size")
    p.add_argument("--root", action="append", default=[],
                   help="folder to search for cargo target dirs (repeatable; default ~/biorouter-runs)")
    p.add_argument("--max-depth", type=int, default=8, help="how deep to search each root")
    p.add_argument("--protect", action="append", default=[], help="never touch anything at or under this path")
    p.add_argument("--protect-file", help="one protected path per line, re-read every purge pass")
    p.add_argument("--t1", type=float, default=80, help="tier 1 below this many GiB free")
    p.add_argument("--t2", type=float, default=60, help="tier 2 below this many GiB free")
    p.add_argument("--t3", type=float, default=40, help="tier 3 below this many GiB free")
    p.add_argument("--goal-gib", type=float, default=120, help="stop purging once this much is free")
    p.add_argument("--alert-gib", type=float, default=None,
                   help="disk alert floor (default: t3 with --purge, t2 without)")
    p.add_argument("--lane-gib", type=float, default=40, help="disk one more build lane needs, for lane_room")
    p.add_argument("--cooldown-min", type=float, default=20, help="minimum gap between purge passes at one tier")
    p.add_argument("--obj-age-hours", type=float, default=6, help="tier 1+: .o files older than this")
    p.add_argument("--low-obj-age-hours", type=float, default=1, help="tier 3, idle profiles: .o older than this")
    p.add_argument("--incremental-idle-min", type=float, default=60, help="tier 2+: incremental dirs idle this long")
    p.add_argument("--low-incremental-idle-min", type=float, default=20, help="tier 3: incremental idle this long")
    p.add_argument("--exe-age-hours", type=float, default=6, help="tier 2+: test binaries older than this")
    p.add_argument("--low-exe-age-hours", type=float, default=3, help="tier 3: test binaries older than this")
    p.add_argument("--retire-idle-hours", type=float, default=0,
                   help="tier 2+: delete whole cargo targets idle this long with nothing using them (0 = off)")
    p.add_argument("--retire-root", action="append", default=[],
                   help="only targets under these folders may be retired whole (repeatable; your own lanes)")
    p.add_argument("--include-clones", action="store_true",
                   help="let --retire-idle-hours take targets of full clones (a .git directory) too")
    p.add_argument("--skip-download-caches", action="store_true",
                   help="leave the Homebrew, pip and npm download caches alone")
    p.add_argument("--docker", action="store_true",
                   help="tier 2+: prune dangling images and build cache older than 7 days (never volumes)")
    p.add_argument("--swapout-rate", type=float, default=200, help="swapouts per second that count as paging")
    p.add_argument("--streak", type=int, default=3, help="consecutive samples before a paging or orphan alert")
    p.add_argument("--orphan-cpu", type=float, default=50, help="CPU percent that makes an orphan suspect")
    p.add_argument("--no-orphan-alert", action="store_true", help="log orphans but never alert on them")
    a = p.parse_args(argv)
    if not a.root:
        a.root = ["~/biorouter-runs"]
    if a.dry_run and not a.purge:
        p.error("--dry-run only makes sense with --purge")
    if a.retire_idle_hours and not a.retire_root:
        p.error("--retire-idle-hours needs --retire-root: name the folders that hold your own lanes")
    if not (a.t1 > a.t2 > a.t3 > 0):
        p.error("tiers must satisfy t1 > t2 > t3 > 0")
    if a.goal_gib <= a.t1:
        p.error("--goal-gib must be above --t1")
    return a


def main(argv):
    a = parse_args(argv)
    lock_fd = None
    if a.purge or not a.once:
        lock_fd = take_lock(expand(a.lock))
        if lock_fd is None:
            info = ""
            try:
                with open(expand(a.lock) + ".json") as f:
                    info = f.read().strip()
            except OSError:
                pass
            print(f"another resource guard holds {a.lock}: {info or 'unknown'}; check it with ps -o pid,lstart,command -p <pid>")
            return EXIT_USAGE
    guard = Guard(a)

    def stop(signum, _frame):
        global STOP
        STOP = True

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    guard.log({"event": "start", "pid": os.getpid(), "argv": sys.argv[1:], "purge": a.purge, "dry_run": a.dry_run})
    try:
        while not STOP:
            try:
                s, alerts = guard.cycle()
                guard.errors = 0
            except Exception as exc:  # keep watching through a bad sample, but not forever
                guard.errors += 1
                guard.log({"event": "error", "error": repr(exc)})
                if guard.errors >= 5:
                    print(f"GUARD ERROR: {exc!r} five times in a row; log {guard.log_path}", flush=True)
                    return EXIT_ERROR
                s, alerts = None, []
            if a.once:
                if s is None:
                    print(f"GUARD ERROR: the reading failed; see the last error line in {guard.log_path}")
                    return EXIT_ERROR
                print(summary(s, guard))
                return alerts[0][1] if alerts else 0
            if alerts and not a.keep_running:
                return alerts[0][1]
            end = time.time() + a.interval
            while not STOP and time.time() < end:
                time.sleep(max(0.0, min(1.0, end - time.time())))
        guard.log({"event": "stop", "pid": os.getpid()})
        return 0
    finally:
        unlock(lock_fd)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
