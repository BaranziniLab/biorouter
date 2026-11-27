#!/usr/bin/env python3
"""Self-test for resource-guard.py: every purge rule must delete what it should AND keep what it
must, on a synthetic tree of fake cargo targets in a temporary folder.

It never touches real caches: the guard runs with --skip-download-caches, its own lock and log,
and roots limited to the temporary folder. Helper processes (a lock holder, an open-file holder,
a "running test") are started here and stopped here by pid.

usage: python3 -I test-resource-guard.py [--keep]    (--keep leaves the temp folder for a look)
"""
import fcntl
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
GUARD = os.path.join(HERE, "resource-guard.py")
H = 3600


def touch(path, age_s=0, data=b"x" * 4096, exe=False):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    if exe:
        os.chmod(path, 0o755)
    t = time.time() - age_s
    os.utime(path, (t, t))


def age_dir(path, age_s):
    t = time.time() - age_s
    os.utime(path, (t, t))


def checkout(root, name, full_clone=False):
    wt = os.path.join(root, name)
    os.makedirs(wt)
    if full_clone:
        os.makedirs(os.path.join(wt, ".git"))
    else:
        touch(os.path.join(wt, ".git"), data=b"gitdir: elsewhere\n")
    touch(os.path.join(wt, "Cargo.toml"), data=b"[workspace]\n")
    target = os.path.join(wt, "target")
    touch(os.path.join(target, "CACHEDIR.TAG"), data=b"Signature: 8a477f597d28d172789f06886806bc55\n")
    touch(os.path.join(target, "debug", ".cargo-lock"), data=b"")
    return os.path.join(target, "debug")


def build_tree(root):
    p = {}
    # A: idle worktree, mixed ages.
    a = checkout(root, "wtA")
    p["a_old_o"] = f"{a}/deps/old-1.rcgu.o";          touch(p["a_old_o"], 2 * H)
    p["a_new_o"] = f"{a}/deps/new-1.rcgu.o";          touch(p["a_new_o"])
    p["a_old_exe"] = f"{a}/deps/test_old-aaaa";       touch(p["a_old_exe"], 4 * H, exe=True)
    p["a_new_exe"] = f"{a}/deps/test_new-bbbb";       touch(p["a_new_exe"], exe=True)
    p["a_run_exe"] = f"{a}/deps/test_running-cccc";   touch(p["a_run_exe"], 4 * H, exe=True)
    p["a_rlib"] = f"{a}/deps/libfoo-1.rlib";          touch(p["a_rlib"], 10 * H)
    p["a_dep"] = f"{a}/deps/foo-1.d";                 touch(p["a_dep"], 10 * H)
    p["a_linked"] = f"{a}/deps/linked-dddd";          touch(p["a_linked"], 4 * H, exe=True)
    p["a_top"] = f"{a}/linked"
    os.link(p["a_linked"], p["a_top"])
    p["a_fp"] = f"{a}/.fingerprint/foo-1/lib";        touch(p["a_fp"], 10 * H)
    p["a_inc_old"] = f"{a}/incremental/crate_old-1";  touch(f"{p['a_inc_old']}/s-1/x.bin", H)
    age_dir(f"{p['a_inc_old']}/s-1", H); age_dir(p["a_inc_old"], H)
    p["a_inc_work"] = f"{a}/incremental/crate_work-2"; touch(f"{p['a_inc_work']}/s-2-working/x.bin", H)
    age_dir(f"{p['a_inc_work']}/s-2-working", H); age_dir(p["a_inc_work"], H)
    p["a_inc_new"] = f"{a}/incremental/crate_new-3";  touch(f"{p['a_inc_new']}/s-3/x.bin")
    # B: a build holds the cargo lock.
    b = checkout(root, "wtB")
    p["b_lock"] = f"{b}/.cargo-lock"
    p["b_old8_o"] = f"{b}/deps/old8-1.rcgu.o";        touch(p["b_old8_o"], 8 * H)
    p["b_old2_o"] = f"{b}/deps/old2-1.rcgu.o";        touch(p["b_old2_o"], 2 * H)
    p["b_old_exe"] = f"{b}/deps/test_old-eeee";       touch(p["b_old_exe"], 8 * H, exe=True)
    p["b_inc_old"] = f"{b}/incremental/crate_old-1";  touch(f"{p['b_inc_old']}/s-1/x.bin", 2 * H)
    age_dir(f"{p['b_inc_old']}/s-1", 2 * H); age_dir(p["b_inc_old"], 2 * H)
    # C: protected.
    c = checkout(root, "wtC")
    p["c_root"] = os.path.dirname(os.path.dirname(c))
    p["c_old_o"] = f"{c}/deps/old-1.rcgu.o";          touch(p["c_old_o"], 8 * H)
    # D: idle for hours: the whole target goes.
    d = checkout(root, "wtD")
    p["d_target"] = os.path.dirname(d)
    for name in ("libd-1.rlib", "d-1.d", "old-1.rcgu.o"):
        touch(f"{d}/deps/{name}", 3 * H)
    touch(f"{d}/.fingerprint/d-1/lib", 3 * H); age_dir(f"{d}/.fingerprint/d-1", 3 * H)
    # E: idle for hours, but a process holds a file open in it.
    e = checkout(root, "wtE")
    p["e_target"] = os.path.dirname(e)
    p["e_open"] = f"{e}/deps/libe-1.rlib";            touch(p["e_open"], 3 * H)
    touch(f"{e}/.fingerprint/e-1/lib", 3 * H); age_dir(f"{e}/.fingerprint/e-1", 3 * H)
    # F: a full clone (a .git directory), idle for hours: pruned, never retired by default.
    f = checkout(root, "cloneF", full_clone=True)
    p["f_target"] = os.path.dirname(f)
    p["f_rlib"] = f"{f}/deps/libf-1.rlib";            touch(p["f_rlib"], 3 * H)
    p["f_old_o"] = f"{f}/deps/old-1.rcgu.o";          touch(p["f_old_o"], 3 * H)
    # G: a node_modules tree that happens to hold a cargo target: never searched.
    g = os.path.join(root, "pkg", "node_modules", "x", "target", "debug")
    touch(os.path.join(os.path.dirname(g), "CACHEDIR.TAG"))
    touch(os.path.join(g, ".cargo-lock"), data=b"")
    p["g_old_o"] = f"{g}/deps/old-1.rcgu.o";          touch(p["g_old_o"], 8 * H)
    return p


def guard(root, extra, env=None):
    log = os.path.join(root, "guard.jsonl")
    cmd = [sys.executable, "-I", GUARD, "--once", "--root", root, "--volume", root, "--log", log,
           "--lock", os.path.join(root, "guard.lock"), "--skip-download-caches",
           "--t1", "1000000", "--t2", "999999", "--t3", "999998", "--goal-gib", "1000001"] + extra
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600, env=env)
    return r.returncode, r.stdout + r.stderr, log


def main():
    keep = "--keep" in sys.argv
    root = os.path.realpath(tempfile.mkdtemp(prefix="resource-guard-test-"))
    helpers = []
    failures = []

    def check(name, cond):
        print(f"{'ok  ' if cond else 'FAIL'} {name}")
        if not cond:
            failures.append(name)

    try:
        p = build_tree(root)
        snapshot = {k: os.path.exists(v) for k, v in p.items()}

        # Helpers, all stopped by pid in the finally block.
        helpers.append(subprocess.Popen(
            [sys.executable, "-I", "-c",
             "import fcntl,os,sys,time; fd=os.open(sys.argv[1], os.O_RDWR); "
             "fcntl.flock(fd, fcntl.LOCK_EX); print('locked', flush=True); time.sleep(300)", p["b_lock"]],
            stdout=subprocess.PIPE, text=True))
        assert helpers[-1].stdout.readline().strip() == "locked"
        # Holds a file open in wtE. The path goes in through the environment, not argv, so only
        # the guard's lsof check (not its command-line check) can see it.
        helpers.append(subprocess.Popen(
            [sys.executable, "-I", "-c",
             "import os,time; f=open(os.environ['HELD']); print('open', flush=True); time.sleep(300)"],
            stdout=subprocess.PIPE, text=True, env=dict(os.environ, HELD=p["e_open"])))
        assert helpers[-1].stdout.readline().strip() == "open"
        # Stands in for a running test binary: its command line names the file. (A plain
        # `sh -c 'sleep 300' sh <path>` execs sleep and drops the path, so it cannot stand in.)
        helpers.append(subprocess.Popen(
            [sys.executable, "-I", "-c", "import time; time.sleep(300)", p["a_run_exe"]]))
        time.sleep(0.5)
        ps = subprocess.run(["ps", "-axww", "-o", "command="], capture_output=True, text=True).stdout
        assert p["a_run_exe"] in ps, "the running-test stand-in does not show its path to ps"

        # 1. Read-only: nothing changes.
        rc, out, _ = guard(root, [])
        check("read-only run alerts on disk (exit 3)", rc == 3 and "ALERT disk" in out)
        check("read-only run deletes nothing", all(os.path.exists(v) == snapshot[k] for k, v in p.items()))

        # 2. Dry run: nothing changes, candidates are logged.
        rc, out, log = guard(root, ["--purge", "--dry-run", "--retire-idle-hours", "1", "--retire-root", root])
        check("dry run deletes nothing", all(os.path.exists(v) == snapshot[k] for k, v in p.items()))
        recs = [json.loads(l) for l in open(log)]
        check("dry run logs would-be purges", any(r.get("event") == "purge" and r.get("dry_run") for r in recs))

        # 3. A second purging guard is refused while the lock is held.
        holder = subprocess.Popen(
            [sys.executable, "-I", "-c",
             "import fcntl,os,sys,time; fd=os.open(sys.argv[1], os.O_RDWR|os.O_CREAT); "
             "fcntl.flock(fd, fcntl.LOCK_EX); print('locked', flush=True); time.sleep(300)",
             os.path.join(root, "guard.lock")], stdout=subprocess.PIPE, text=True)
        helpers.append(holder)
        assert holder.stdout.readline().strip() == "locked"
        rc, out, _ = guard(root, ["--purge"])
        check("second purging guard exits 2", rc == 2 and "another resource guard" in out)
        holder.kill(); holder.wait()

        # 4. Real purge at tier 3 with whole-target retirement.
        rc, out, log = guard(root, ["--purge", "--protect", p["c_root"], "--retire-idle-hours", "1", "--retire-root", root])
        check("purge run still alerts (exit 3)", rc == 3)
        gone = lambda k: not os.path.exists(p[k])
        kept = lambda k: os.path.exists(p[k])
        check("idle: old .o deleted", gone("a_old_o"))
        check("idle: new .o kept", kept("a_new_o"))
        check("idle: old test binary deleted", gone("a_old_exe"))
        check("idle: new test binary kept", kept("a_new_exe"))
        check("idle: running test binary kept", kept("a_run_exe"))
        check("idle: hard-linked binary copy kept", kept("a_linked") and kept("a_top"))
        check("idle: rlib, .d and fingerprint kept", kept("a_rlib") and kept("a_dep") and kept("a_fp"))
        check("idle: idle incremental deleted", gone("a_inc_old"))
        check("idle: -working incremental kept", kept("a_inc_work"))
        check("idle: fresh incremental kept", kept("a_inc_new"))
        check("busy: 8 h single-link .o deleted", gone("b_old8_o"))
        check("busy: 2 h .o kept", kept("b_old2_o"))
        check("busy: test binary kept", kept("b_old_exe"))
        check("busy: incremental kept", kept("b_inc_old"))
        check("protected: untouched", kept("c_old_o"))
        check("idle target retired whole", gone("d_target"))
        check("target with an open file not retired", kept("e_target") and kept("e_open"))
        check("full clone pruned but not retired", kept("f_target") and kept("f_rlib") and gone("f_old_o"))
        check("node_modules never searched", kept("g_old_o"))
        recs = [json.loads(l) for l in open(log)]
        purges = [r for r in recs if r.get("event") == "purge" and not r.get("dry_run")]
        check("every deletion logged with a size", purges and all("single_link_gib" in r for r in purges))
        check("a purge_pass summary was logged", any(r.get("event") == "purge_pass" for r in recs))

        # 5. Fail closed: with lsof unreachable, nothing that needs "no build is using this" goes.
        root2 = os.path.join(root, "failclosed")
        os.makedirs(root2)
        q = build_tree(root2)
        path = "/bin:/usr/bin"
        assert shutil.which("lsof", path=path) is None, "lsof is still reachable; the test cannot fail"
        rc, out, log = guard(root2, ["--purge", "--retire-idle-hours", "1", "--retire-root", root2], env=dict(os.environ, PATH=path))
        kept2 = lambda k: os.path.exists(q[k])
        check("no lsof: idle-only rules skipped", kept2("a_old_exe") and kept2("a_inc_old") and kept2("a_old_o"))
        check("no lsof: no target retired", kept2("d_target"))
        check("no lsof: busy-safe old objects still go", not kept2("b_old8_o"))
        recs = [json.loads(l) for l in open(log)]
        check("no lsof: the skip is logged", any(r.get("event") == "skip" and "lsof" in r.get("why", "") for r in recs))
    finally:
        for h in helpers:
            if h.poll() is None:
                h.kill()
                h.wait()
        if keep:
            print(f"left {root}")
        else:
            shutil.rmtree(root, ignore_errors=True)
    print(f"\n{len(failures)} failed" if failures else "\nall checks passed")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
