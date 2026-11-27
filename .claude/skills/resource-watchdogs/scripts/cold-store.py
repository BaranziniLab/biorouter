#!/usr/bin/env python3
"""Park cold folders on the owner-approved external disk as verified tar archives.

For each source: refuse it if anything was written there recently, if a process has a file or
its working directory inside it, or if it holds credential-looking files. Otherwise write ONE tar
file (an exFAT disk wastes a whole cluster per small file, and tar keeps modes and symlinks),
list the archive and compare its entry count with the source, hash the archive by reading it back
from the external disk, and append a line to MANIFEST.jsonl. The source is removed only with
--remove-source, and only after every check passed. Anything that fails a step stays in place.

usage:
  python3 -I cold-store.py SRC [SRC ...] --dest-root /Volumes/<disk>/biorouter-backup/<campaign>
                           [--remove-source] [--note TEXT] [--min-idle-min 20] [--dry-run]

Restore: tar -xf <archive> -C <parent folder>   (check the sha256 in MANIFEST.jsonl first)
Exit codes: 0 every source archived (or skipped as already archived); 1 any source failed;
2 usage error.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time

# Credential-looking names: they never go to an external disk without the owner's word.
SECRET_NAME = re.compile(
    r"^(secrets?\.ya?ml|\.env(\..+)?|.+\.env|.*live-test\.env.*|id_(rsa|ed25519|ecdsa|dsa)|"
    r".+\.(pem|p12|pfx|key|keystore)|credentials(\.json)?|\.netrc|user-action-key\.json)$",
    re.I,
)


def newest_mtime(path):
    newest = os.lstat(path).st_mtime
    for root, dirs, files in os.walk(path):
        for name in dirs + files:
            try:
                newest = max(newest, os.lstat(os.path.join(root, name)).st_mtime)
            except OSError:
                pass
    return newest


def count_entries(path):
    total = 1
    for _root, dirs, files in os.walk(path):
        total += len(dirs) + len(files)
    return total


def secret_files(path):
    found = []
    for root, _dirs, files in os.walk(path):
        found += [os.path.join(root, f) for f in files if SECRET_NAME.match(f)]
    return found


def in_use(path):
    """True when a process has a file or its cwd inside path; None when lsof failed."""
    try:
        r = subprocess.run(["lsof", "-w", "-nP", "-Fn"], capture_output=True, text=True, timeout=180)
    except (OSError, subprocess.SubprocessError):
        return None
    if r.returncode not in (0, 1) or (r.returncode == 1 and r.stderr.strip()):
        return None
    prefix = path.rstrip("/") + "/"
    return any(l[1:] == path or l[1:].startswith(prefix) for l in r.stdout.splitlines() if l.startswith("n/"))


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(8 * 1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def du_bytes(path):
    out = subprocess.run(["du", "-sk", path], capture_output=True, text=True).stdout.split()
    return int(out[0]) * 1024 if out else 0


def main(argv):
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("src", nargs="+", help="folders (or files) to archive")
    p.add_argument("--dest-root", required=True, help="folder on the external disk")
    p.add_argument("--remove-source", action="store_true", help="delete each source once its archive verifies")
    p.add_argument("--note", default="", help="why these were parked, recorded in the manifest")
    p.add_argument("--min-idle-min", type=float, default=20, help="refuse sources written to this recently")
    p.add_argument("--allow-secrets", action="store_true", help="the owner said credential files may go too")
    p.add_argument("--allow-any-dest", action="store_true", help="destination need not be under /Volumes/")
    p.add_argument("--dry-run", action="store_true", help="run the refusal checks only")
    a = p.parse_args(argv)

    dest_root = os.path.realpath(os.path.expanduser(a.dest_root))
    parts = dest_root.split(os.sep)
    if not a.allow_any_dest:
        if len(parts) < 3 or parts[1] != "Volumes" or not os.path.ismount(os.sep.join(parts[:3])):
            print(f"refusing: {dest_root} is not on a mounted external volume under /Volumes/")
            return 2
    os.makedirs(dest_root, exist_ok=True)
    manifest = os.path.join(dest_root, "MANIFEST.jsonl")
    home = os.path.expanduser("~")
    failed = moved = 0
    freed = 0
    for raw in a.src:
        src = os.path.realpath(os.path.expanduser(raw))
        if os.path.islink(raw) or not os.path.exists(src):
            print(f"FAIL {raw}: missing or a symlink")
            failed += 1
            continue
        if src == dest_root or src.startswith(dest_root + "/"):
            print(f"FAIL {src}: already on the destination")
            failed += 1
            continue
        idle_min = (time.time() - newest_mtime(src)) / 60
        if idle_min < a.min_idle_min:
            print(f"FAIL {src}: written {idle_min:.0f} min ago; something may still use it")
            failed += 1
            continue
        busy = in_use(src)
        if busy is None or busy:
            print(f"FAIL {src}: {'lsof failed, cannot prove it is unused' if busy is None else 'a process uses it'}")
            failed += 1
            continue
        secrets = [] if a.allow_secrets else (secret_files(src) if os.path.isdir(src) else
                                             ([src] if SECRET_NAME.match(os.path.basename(src)) else []))
        if secrets:
            print(f"FAIL {src}: holds credential-looking files; remove them or ask the owner first:")
            for s in secrets[:20]:
                print(f"    {s}")
            failed += 1
            continue
        rel = os.path.relpath(src, home) if src.startswith(home + "/") else src.lstrip("/")
        dest = os.path.join(dest_root, rel.replace("/", "__") + ".tar")
        if os.path.exists(dest):
            print(f"skip {src}: {dest} already exists")
            continue
        size = du_bytes(src)
        if a.dry_run:
            print(f"would archive {src} ({size / 2**30:.2f} GiB) -> {dest}")
            continue
        if shutil.disk_usage(dest_root).free < size * 1.05:
            print(f"FAIL {src}: destination reports less free space than {size / 2**30:.2f} GiB")
            failed += 1
            continue
        parent, name = os.path.split(src)
        tmp = dest + ".partial"
        made = subprocess.run(["tar", "-cf", tmp, "-C", parent, name], capture_output=True, text=True)
        if made.returncode != 0:
            print(f"FAIL tar {src}: rc {made.returncode}: {made.stderr.strip()[:300]}")
            if os.path.exists(tmp):
                os.remove(tmp)
            failed += 1
            continue
        listed = subprocess.run(["tar", "-tf", tmp], capture_output=True, text=True)
        archived = len([l for l in listed.stdout.splitlines() if l.strip()])
        expected = count_entries(src) if os.path.isdir(src) else 1
        if listed.returncode != 0 or archived != expected:
            print(f"FAIL verify {src}: archive lists {archived} entries, source has {expected}; kept in place")
            os.remove(tmp)
            failed += 1
            continue
        os.rename(tmp, dest)
        digest = sha256(dest)  # read back from the external disk, not from memory
        with open(manifest, "a") as m:
            m.write(json.dumps({"source": src, "archive": dest, "entries": archived, "source_bytes": size,
                                "archive_bytes": os.path.getsize(dest), "archive_sha256": digest,
                                "archived_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
                                "source_removed": a.remove_source, "note": a.note}) + "\n")
        if a.remove_source:
            subprocess.run(["chmod", "-R", "u+w", src], capture_output=True)
            if os.path.isdir(src):
                shutil.rmtree(src)
            else:
                os.remove(src)
            freed += size
        moved += 1
        print(f"archived {src} ({size / 2**30:.2f} GiB, {archived} entries, sha256 {digest[:12]}) -> {dest}"
              f"{'; source removed' if a.remove_source else '; source kept'}")
    print(f"SUMMARY archived {moved}, failed {failed}, freed {freed / 2**30:.2f} GiB; manifest {manifest}")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
