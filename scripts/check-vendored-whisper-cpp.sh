#!/usr/bin/env bash
# The vendored whisper.cpp subset (the local dictation engine, DD-3) must match its
# manifest in both directions, survive a fresh checkout on any platform unchanged, and
# name a pin that contains the CVE-2026-110810 loader fix.
#
# Usage:
#   scripts/check-vendored-whisper-cpp.sh                     verify (CI `guards`, `just check-everything`)
#   scripts/check-vendored-whisper-cpp.sh --self-test         prove the check fails when it must
#   scripts/check-vendored-whisper-cpp.sh --refresh <archive> replace source/ from upstream at the pin
#   scripts/check-vendored-whisper-cpp.sh --write-manifest    re-hash the tree already vendored
#
# `--refresh` takes GitHub's archive of the pinned commit
# (https://github.com/ggml-org/whisper.cpp/archive/<commit>.tar.gz) or a clone that has
# the commit. Either way it reads the commit id git embeds in the archive and refuses one
# that is not pin.json's, so the tree cannot come from a different commit than the pin
# says. Edit pin.json first; vendor/whisper.cpp/README.md has the whole procedure.
#
# What "verify" checks, and why each one exists:
#   1. pin.json has `upstream`, a 40 digit `commit`, `contains` with 60c0be6, `ggml` and
#      `date`, and `ggml` equals the version in source/ggml/CMakeLists.txt.
#   2. source-manifest.json names the same upstream and commit, and lists every file under
#      source/ with its size and SHA-256. Checked BOTH ways: a file nobody recorded is
#      refused as loudly as an edited or missing one, because it would be build input
#      nobody reviewed. pin.json and source-manifest.json must each be a JSON object: a
#      file that parses to `null` or a list is refused by name, because it would otherwise
#      skip every check that reads it and let an edited tree pass.
#   3. Every file lies inside the vendored subset (only what the build reads, plus four
#      fixtures), so a stray file OUTSIDE the subset (for example under tests/ or
#      examples/) cannot be blessed by regenerating the manifest. Inside the subset it can:
#      there, a regenerated manifest is a change to review in the diff of
#      source-manifest.json.
#   4. No file is gitignored. The Copilot helper's vendored tree, vendor/computer-use/, hit
#      15 ignored files on its first attempt; an ignored file verifies on the machine that
#      vendored it and never reaches a fresh clone.
#   5. Every file under source/ is exempt from end-of-line conversion (`-text` in
#      .gitattributes), or a Windows checkout rewrites bytes nobody edited and the
#      manifest fails on a clean tree.
#   6. No symbolic link and no upstream .gitignore, .gitattributes or .gitmodules under
#      source/: those would change how this repository's own git treats the tree.
#   7. patches/ holds only *.patch files and its README, and vendor/whisper.cpp/LICENSE is
#      upstream's LICENSE byte for byte.
#
# Python 3 is the only requirement beyond git; the program below uses the standard
# library alone.
set -euo pipefail
cd "$(dirname "$0")/.."
exec python3 -I - "$@" <<'PY'
import argparse
import datetime
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

ROOT = Path.cwd()
SCRIPT_REL = "scripts/check-vendored-whisper-cpp.sh"
VENDOR_REL = "vendor/whisper.cpp"
VENDOR = ROOT / VENDOR_REL
SOURCE = VENDOR / "source"
MANIFEST = VENDOR / "source-manifest.json"
PIN = VENDOR / "pin.json"
PATCHES = VENDOR / "patches"
SCHEMA = 1

# DD-3: every pin must contain the loader fix for CVE-2026-110810,
# 60c0be6ac8fa71b1a2ae2dd938a31a34a508e774 "whisper : add check for ttype in whisper and
# parakeet (#4091)".
REQUIRED_CONTAINS = ("60c0be6",)

# Only what the build reads. A directory entry ends with "/".
INCLUDED = (
    "CMakeLists.txt",
    "LICENSE",
    "cmake/",
    "include/",
    "src/",
    "ggml/CMakeLists.txt",
    "ggml/cmake/",
    "ggml/include/",
    "ggml/src/",
    "examples/miniaudio.h",
)
# Upstream's own test assets, used by the offline tests and the worker's self test.
FIXTURES = (
    "models/for-tests-ggml-base.en.bin",
    "models/for-tests-ggml-tiny.bin",
    "models/for-tests-silero-v6.2.0-ggml.bin",
    "samples/jfk.wav",
)
# ggml enters a backend folder only when its GGML_<BACKEND> option is on
# (`ggml_add_backend` in ggml/src/CMakeLists.txt), and the build turns on CPU everywhere
# and Metal on macOS arm64. Every other backend folder is left out.
KEPT_BACKENDS = ("ggml-cpu", "ggml-metal")
BACKEND_REASON = (
    "a ggml backend the build never turns on; ggml enters a backend folder only when its "
    "GGML_<BACKEND> option is on, and the build turns on only CPU and Metal"
)
GIT_SETTINGS = (".gitignore", ".gitattributes", ".gitmodules")
GIT_SETTINGS_REASON = (
    "upstream's own git settings; vendored, they would change how this repository "
    "treats the tree"
)
VENDOR_FILES = ("LICENSE", "NOTICE", "README.md", "pin.json", "source-manifest.json")


def backend_folder(rel):
    parts = rel.split("/")
    if len(parts) > 3 and parts[0] == "ggml" and parts[1] == "src" and parts[2].startswith("ggml-"):
        return parts[2]
    return None


def inside_included(rel):
    if rel in FIXTURES:
        return True
    for entry in INCLUDED:
        if (entry.endswith("/") and rel.startswith(entry)) or rel == entry:
            return True
    return False


def in_subset(rel):
    if not inside_included(rel):
        return False
    if rel.split("/")[-1] in GIT_SETTINGS:
        return False
    folder = backend_folder(rel)
    return folder is None or folder in KEPT_BACKENDS


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def walk(base):
    """(files, links): every regular file under base as {rel: bytes length and digest},
    and every symbolic link, without following any."""
    files, links = {}, []
    if not base.is_dir():
        return files, links
    for dirpath, dirnames, filenames in os.walk(base, followlinks=False):
        here = Path(dirpath)
        for name in list(dirnames):
            if (here / name).is_symlink():
                links.append(str((here / name).relative_to(base)).replace(os.sep, "/"))
        for name in filenames:
            path = here / name
            rel = str(path.relative_to(base)).replace(os.sep, "/")
            if path.is_symlink():
                links.append(rel)
                continue
            data = path.read_bytes()
            files[rel] = {"size": len(data), "sha256": sha256(data)}
    return files, sorted(links)


def tree_digest(entries):
    """One digest over names, sizes and contents, so a rename or an edit both move it."""
    joined = "".join(f"{e['path']}\0{e['size']}\0{e['sha256']}\n" for e in entries)
    return sha256(joined.encode())


def load_json(path, problems, label):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        problems.append(f"{label} is missing: {path.relative_to(ROOT)}")
    except (ValueError, UnicodeDecodeError) as error:
        problems.append(f"{label} is not valid JSON: {error}")
    return None


def load_object(path, problems, label, remedy=""):
    """load_json, plus a named problem when the file parses to anything but an object
    (`null`, a list, a number). Every check that reads the file is gated on it being an
    object, so without this a `null` manifest skipped the hash checks and passed an
    edited tree. Returns the object, or None after recording why."""
    before = len(problems)
    value = load_json(path, problems, label)
    if isinstance(value, dict):
        return value
    if len(problems) == before:
        problems.append(f"{label} must be a JSON object{remedy}")
    return None


def ggml_version_in_tree():
    path = SOURCE / "ggml/CMakeLists.txt"
    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return None
    parts = []
    for name in ("MAJOR", "MINOR", "PATCH"):
        match = re.search(rf"set\(\s*GGML_VERSION_{name}\s+(\d+)\s*\)", text)
        if not match:
            return None
        parts.append(match.group(1))
    return ".".join(parts)


def pin_problems(pin):
    problems = []
    if not isinstance(pin, dict):
        return ["pin.json must be a JSON object"]
    upstream = pin.get("upstream")
    if not (isinstance(upstream, str) and re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", upstream)):
        problems.append("pin.json: `upstream` must name the GitHub repository as owner/name")
    commit = pin.get("commit")
    if not (isinstance(commit, str) and re.fullmatch(r"[0-9a-f]{40}", commit)):
        problems.append("pin.json: `commit` must be a full 40 digit lowercase hex commit id")
    contains = pin.get("contains")
    if not (isinstance(contains, list) and all(isinstance(c, str) and re.fullmatch(r"[0-9a-f]{7,40}", c) for c in contains)):
        problems.append("pin.json: `contains` must be a list of commit ids (7 to 40 hex digits)")
    else:
        for required in REQUIRED_CONTAINS:
            if not any(c.startswith(required) for c in contains):
                problems.append(
                    f"pin.json: `contains` must list {required}, the CVE-2026-110810 loader fix (DD-3); "
                    "prove the new pin contains it before adding it"
                )
    ggml = pin.get("ggml")
    if not (isinstance(ggml, str) and re.fullmatch(r"\d+\.\d+\.\d+", ggml)):
        problems.append("pin.json: `ggml` must be the ggml version, as MAJOR.MINOR.PATCH")
    else:
        found = ggml_version_in_tree()
        if found is not None and found != ggml:
            problems.append(
                f"pin.json: `ggml` says {ggml} but source/ggml/CMakeLists.txt says {found}"
            )
    date = pin.get("date")
    valid_date = isinstance(date, str) and re.fullmatch(r"\d{4}-\d{2}-\d{2}", date)
    if valid_date:
        try:
            datetime.date.fromisoformat(date)
        except ValueError:
            valid_date = False
    if not valid_date:
        problems.append("pin.json: `date` must be the pinned commit's date, as YYYY-MM-DD")
    return problems


def build_manifest(pin, excluded):
    files, _ = walk(SOURCE)
    entries = [{"path": rel, "size": files[rel]["size"], "sha256": files[rel]["sha256"]} for rel in sorted(files)]
    return {
        "schema_version": SCHEMA,
        "upstream": pin["upstream"],
        "commit": pin["commit"],
        "included": list(INCLUDED) + list(FIXTURES),
        "kept_backends": list(KEPT_BACKENDS),
        "excluded": excluded,
        "file_count": len(entries),
        "total_bytes": sum(e["size"] for e in entries),
        "tree_sha256": tree_digest(entries),
        "files": entries,
    }


def git(args, paths=None):
    payload = None if paths is None else ("\0".join(paths) + "\0").encode()
    return subprocess.run(["git", *args], input=payload, capture_output=True, cwd=ROOT)


def nul_fields(output):
    fields = output.decode("utf-8", errors="replace").split("\0")
    if fields and fields[-1] == "":
        fields.pop()
    return fields


def git_problems(source_files):
    probe = git(["rev-parse", "--is-inside-work-tree"])
    if probe.returncode != 0 or probe.stdout.strip() != b"true":
        return [f"{ROOT} is not a git work tree; the committable checks need git"]
    problems = []
    vendor_files, _ = walk(VENDOR)
    every = [f"{VENDOR_REL}/{rel}" for rel in sorted(vendor_files)]

    ignored = git(["check-ignore", "-z", "--stdin", "--no-index"], every)
    if ignored.returncode not in (0, 1):
        problems.append("git check-ignore failed: " + ignored.stderr.decode(errors="replace").strip())
    else:
        hits = nul_fields(ignored.stdout)
        if hits:
            verbose = nul_fields(git(["check-ignore", "-z", "-v", "--stdin", "--no-index"], hits).stdout)
            shown = []
            for i in range(0, len(verbose) - 3, 4):
                source, line, pattern, path = verbose[i:i + 4]
                shown.append(f"{path}  (rule `{pattern}` at {source}:{line})")
            shown = shown or hits
            problems.append(
                f"{len(hits)} vendored file(s) are matched by an ignore rule (ignored), so they would "
                "never reach a fresh clone; add a negation for vendor/whisper.cpp/ to .gitignore:\n    "
                + "\n    ".join(shown[:20])
            )

    sources = [f"{VENDOR_REL}/source/{rel}" for rel in sorted(source_files)]
    if sources:
        attrs = git(["check-attr", "-z", "--stdin", "text"], sources)
        if attrs.returncode != 0:
            problems.append("git check-attr failed: " + attrs.stderr.decode(errors="replace").strip())
        else:
            fields = nul_fields(attrs.stdout)
            unpinned = [
                f"{fields[i]} (text: {fields[i + 2]})"
                for i in range(0, len(fields) - 2, 3)
                if fields[i + 2] != "unset"
            ]
            if unpinned:
                problems.append(
                    f"{len(unpinned)} file(s) under source/ are not exempt from end-of-line conversion, so a "
                    "Windows checkout would rewrite them; .gitattributes needs "
                    "`vendor/whisper.cpp/source/** -text`:\n    " + "\n    ".join(unpinned[:20])
                )
    return problems


def verify():
    problems = []
    pin = load_object(PIN, problems, "pin.json")
    if pin is not None:
        problems += pin_problems(pin)
    manifest = load_object(MANIFEST, problems, "source-manifest.json", "; regenerate it with --refresh")
    if not SOURCE.is_dir():
        problems.append(f"the vendored tree is missing: {SOURCE.relative_to(ROOT)}")
    files, links = walk(SOURCE)

    if isinstance(manifest, dict):
        if manifest.get("schema_version") != SCHEMA:
            problems.append(f"source-manifest.json: unsupported schema_version {manifest.get('schema_version')!r}")
        if isinstance(pin, dict):
            for key in ("upstream", "commit"):
                if manifest.get(key) != pin.get(key):
                    problems.append(
                        f"source-manifest.json names {key} {manifest.get(key)!r} but pin.json names "
                        f"{pin.get(key)!r}; vendor the pinned commit with --refresh"
                    )
        if manifest.get("included") != list(INCLUDED) + list(FIXTURES) or manifest.get("kept_backends") != list(KEPT_BACKENDS):
            problems.append(
                "source-manifest.json records a different subset than this script vendors; "
                "regenerate it with --refresh"
            )
        recorded = {}
        for entry in manifest.get("files") or []:
            if isinstance(entry, dict) and isinstance(entry.get("path"), str):
                recorded[entry["path"]] = entry
        missing = sorted(set(recorded) - set(files))
        extra = sorted(set(files) - set(recorded))
        changed = sorted(
            rel for rel in set(recorded) & set(files)
            if recorded[rel].get("size") != files[rel]["size"] or recorded[rel].get("sha256") != files[rel]["sha256"]
        )
        if missing:
            problems.append(f"{len(missing)} file(s) listed in the manifest but missing:\n    " + "\n    ".join(missing[:20]))
        if extra:
            problems.append(f"{len(extra)} file(s) present but unrecorded in the manifest:\n    " + "\n    ".join(extra[:20]))
        if changed:
            problems.append(f"{len(changed)} file(s) differ from the manifest (changed):\n    " + "\n    ".join(changed[:20]))
        if not (missing or extra or changed):
            entries = [{"path": rel, **files[rel]} for rel in sorted(files)]
            if manifest.get("tree_sha256") != tree_digest(entries):
                problems.append("source-manifest.json: tree_sha256 does not match the tree")
            if manifest.get("file_count") != len(entries) or manifest.get("total_bytes") != sum(e["size"] for e in entries):
                problems.append("source-manifest.json: file_count or total_bytes does not match the tree")

    outside = sorted(rel for rel in files if not in_subset(rel))
    if outside:
        problems.append(
            f"{len(outside)} file(s) lie outside the vendored subset (only what the build reads, plus the "
            "four fixtures):\n    " + "\n    ".join(outside[:20])
        )
    settings = sorted(rel for rel in files if rel.split("/")[-1] in GIT_SETTINGS)
    if settings:
        problems.append("upstream git settings files under source/ (" + GIT_SETTINGS_REASON + "):\n    " + "\n    ".join(settings))
    if links:
        problems.append("symlinks under source/ (the vendored tree holds regular files only):\n    " + "\n    ".join(links[:20]))
    for rel in INCLUDED + FIXTURES:
        if not rel.endswith("/") and rel not in files:
            problems.append(f"source/{rel} is missing; the build or the offline tests read it")

    for name in VENDOR_FILES:
        if not (VENDOR / name).is_file():
            problems.append(f"{VENDOR_REL}/{name} is missing")
    if (VENDOR / "LICENSE").is_file() and (SOURCE / "LICENSE").is_file():
        if (VENDOR / "LICENSE").read_bytes() != (SOURCE / "LICENSE").read_bytes():
            problems.append(f"{VENDOR_REL}/LICENSE is not upstream's source/LICENSE byte for byte")
    if PATCHES.is_dir():
        stray = sorted(
            str(p.relative_to(PATCHES)).replace(os.sep, "/")
            for p in PATCHES.rglob("*")
            if not p.is_dir() and not (p.parent == PATCHES and (p.name == "README.md" or p.name.endswith(".patch")))
        )
        if stray:
            problems.append("patches/ may hold only *.patch files and README.md:\n    " + "\n    ".join(stray))
    else:
        problems.append(f"{VENDOR_REL}/patches/ is missing")
    unknown = sorted(
        p.name for p in VENDOR.iterdir()
        if p.name not in VENDOR_FILES + ("source", "patches")
    ) if VENDOR.is_dir() else []
    if unknown:
        problems.append(f"unexpected entries in {VENDOR_REL}/: " + ", ".join(unknown))

    problems += git_problems(files)
    return problems, files, pin


def report(problems, files, pin):
    # The success line reads the pin verify() already checked, never a second read of
    # the file: a pin that is not an object is a named problem, not a traceback here.
    if not problems and not isinstance(pin, dict):
        problems = ["pin.json must be a JSON object"]
    if problems:
        print(f"{len(problems)} problem(s) with the vendored whisper.cpp tree ({VENDOR_REL}):\n", file=sys.stderr)
        for problem in problems:
            print(f"::error::{problem}\n", file=sys.stderr)
        return 1
    total = sum(f["size"] for f in files.values())
    print(
        f"vendored whisper.cpp OK: {len(files)} files, {total} B at {pin['commit'][:12]} "
        f"(ggml {pin['ggml']}), contains {', '.join(pin['contains'])}; committable and EOL pinned"
    )
    return 0


def open_archive(origin, commit):
    path = Path(origin)
    if path.is_dir():
        archived = subprocess.run(
            ["git", "archive", "--format=tar", f"--prefix=whisper.cpp-{commit}/", commit],
            cwd=path, capture_output=True,
        )
        if archived.returncode != 0:
            raise SystemExit(f"{path} cannot archive {commit}: " + archived.stderr.decode(errors="replace").strip())
        return tarfile.open(fileobj=io.BytesIO(archived.stdout), mode="r:")
    return tarfile.open(path, mode="r:*")


def refresh(origin):
    problems = []
    pin = load_object(PIN, problems, "pin.json")
    if pin is not None:
        problems += [p for p in pin_problems(pin) if "`ggml` says" not in p]
    if problems:
        raise SystemExit("fix pin.json before vendoring:\n  " + "\n  ".join(problems))
    commit = pin["commit"]
    with open_archive(origin, commit) as tar:
        members = tar.getmembers()
        embedded = tar.pax_headers.get("comment")
        if embedded != commit:
            raise SystemExit(
                f"the archive was made from commit {embedded!r}, not pin.json's {commit}; "
                f"download https://github.com/{pin['upstream']}/archive/{commit}.tar.gz"
            )
        staging = VENDOR / "source.new"
        if staging.exists():
            shutil.rmtree(staging)
        staging.mkdir(parents=True)
        dropped_backends, dropped_settings = set(), set()
        for member in members:
            head, _, rel = member.name.partition("/")
            rel = rel.rstrip("/")
            if not rel or member.isdir():
                continue
            segments = rel.split("/")
            if rel.startswith("/") or any(s in ("", ".", "..") for s in segments):
                raise SystemExit(f"refusing an archive member with an unsafe path: {member.name}")
            if not inside_included(rel):
                continue
            folder = backend_folder(rel)
            if folder is not None and folder not in KEPT_BACKENDS:
                dropped_backends.add(f"ggml/src/{folder}/")
                continue
            if segments[-1] in GIT_SETTINGS:
                dropped_settings.add(rel)
                continue
            if not member.isfile():
                raise SystemExit(f"{rel} is not a regular file in the archive; the vendored subset takes regular files only")
            target = staging / rel
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(tar.extractfile(member).read())
    absent = [rel for rel in INCLUDED + FIXTURES if not rel.endswith("/") and not (staging / rel).is_file()]
    absent += [rel for rel in INCLUDED if rel.endswith("/") and not (staging / rel).is_dir()]
    if absent:
        shutil.rmtree(staging)
        raise SystemExit("the archive lacks parts of the subset: " + ", ".join(absent))
    if SOURCE.exists():
        shutil.rmtree(SOURCE)
    staging.rename(SOURCE)
    excluded = [{"path_prefix": prefix, "reason": BACKEND_REASON} for prefix in sorted(dropped_backends)]
    excluded += [{"path_prefix": rel, "reason": GIT_SETTINGS_REASON} for rel in sorted(dropped_settings)]
    MANIFEST.write_text(json.dumps(build_manifest(pin, excluded), indent=2) + "\n", encoding="utf-8")
    found = ggml_version_in_tree()
    print(f"vendored {commit} from {origin}; ggml in the tree is {found}")
    for entry in excluded:
        print(f"  left out {entry['path_prefix']}")


def write_manifest():
    problems = []
    pin = load_object(PIN, problems, "pin.json")
    if pin is not None:
        problems += [p for p in pin_problems(pin) if "`ggml` says" not in p]
    if problems:
        raise SystemExit("fix pin.json before writing the manifest:\n  " + "\n  ".join(problems))
    previous = {}
    if MANIFEST.is_file():
        try:
            previous = json.loads(MANIFEST.read_text(encoding="utf-8"))
        except ValueError:
            previous = {}
    if not isinstance(previous, dict):
        previous = {}
    MANIFEST.write_text(json.dumps(build_manifest(pin, previous.get("excluded", [])), indent=2) + "\n", encoding="utf-8")
    print(f"wrote {MANIFEST.relative_to(ROOT)}")


def self_test():
    """Each case copies the real vendored tree into a scratch git repository, breaks one
    thing, and runs this script there. The untouched copy must pass and every broken one
    must fail for its own reason (the expected words must appear in the output)."""
    sandbox = Path(tempfile.mkdtemp(prefix="whisper-vendor-selftest-"))
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    # Hermetic git: no system or personal config and no personal ignore file, so only
    # the repository's own rules decide.
    env.update(HOME=str(sandbox / "home"), XDG_CONFIG_HOME=str(sandbox / "xdg"), GIT_CONFIG_NOSYSTEM="1")
    pristine = sandbox / "pristine"
    for rel in (SCRIPT_REL, ".gitattributes", ".gitignore"):
        (pristine / rel).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(ROOT / rel, pristine / rel)
    shutil.copytree(VENDOR, pristine / VENDOR_REL, symlinks=True)

    def vendored(repo, rel):
        return repo / VENDOR_REL / rel

    def edit_pin(change):
        def mutate(repo):
            path = vendored(repo, "pin.json")
            pin = json.loads(path.read_text(encoding="utf-8"))
            change(pin)
            path.write_text(json.dumps(pin, indent=2) + "\n", encoding="utf-8")
        return mutate

    def append(rel, data):
        def mutate(repo):
            with open(vendored(repo, rel), "ab") as handle:
                handle.write(data)
        return mutate

    def overwrite(rel, text):
        def mutate(repo):
            vendored(repo, rel).write_text(text, encoding="utf-8")
        return mutate

    def both(*mutators):
        def mutate(repo):
            for each in mutators:
                each(repo)
        return mutate

    def same_size_edit(repo):
        path = vendored(repo, "source/src/whisper.cpp")
        data = bytearray(path.read_bytes())
        data[0] = ord("/") if data[0] != ord("/") else ord("#")
        path.write_bytes(bytes(data))

    def missing(repo):
        vendored(repo, "source/include/whisper.h").unlink()

    def extra(repo):
        vendored(repo, "source/src/unreviewed.c").write_text("int unreviewed;\n")

    def extra_recorded(repo):
        # tests/ lies outside the subset. A file inside it (under src/, say) that a
        # regenerated manifest records passes: that is a manifest diff to review, and
        # no check here can tell it from a real refresh.
        target = vendored(repo, "source/tests/stray.c")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text("int stray;\n")
        # --write-manifest verifies afterwards and fails here, which is the point: only
        # the second run below is judged.
        subprocess.run(["bash", str(repo / SCRIPT_REL), "--write-manifest"], env=env, capture_output=True)

    def ignored(repo):
        with open(repo / ".gitignore", "a") as handle:
            handle.write("\n*.wav\n")

    def eol_unpinned(repo):
        path = repo / ".gitattributes"
        lines = path.read_text().splitlines(keepends=True)
        path.write_text("".join(l for l in lines if not l.startswith("vendor/whisper.cpp/source/")))

    def stray_patch_file(repo):
        vendored(repo, "patches/notes.txt").write_text("not a patch\n")

    def symlink(repo):
        os.symlink("whisper.cpp", vendored(repo, "source/src/linked.cpp"))

    def git_settings(repo):
        vendored(repo, "source/src/.gitattributes").write_text("* text\n")

    cases = [
        ("the vendored tree as committed passes", lambda repo: None, None),
        ("an edited file fails", append("source/src/whisper.cpp", b"\n"), "changed"),
        ("an edit that keeps the size fails", same_size_edit, "changed"),
        ("a missing file fails", missing, "missing"),
        ("an extra file fails", extra, "unrecorded"),
        ("an extra file outside the subset, blessed by a regenerated manifest, fails", extra_recorded, "outside the vendored subset"),
        (
            "an edited file with source-manifest.json replaced by null fails",
            both(append("source/src/whisper.cpp", b"\n"), overwrite("source-manifest.json", "null\n")),
            "JSON object",
        ),
        (
            "an edited file with source-manifest.json replaced by [] fails",
            both(append("source/src/whisper.cpp", b"\n"), overwrite("source-manifest.json", "[]\n")),
            "JSON object",
        ),
        ("pin.json replaced by null fails", overwrite("pin.json", "null\n"), "JSON object"),
        ("pin.json replaced by [] fails", overwrite("pin.json", "[]\n"), "JSON object"),
        ("a .gitignore matched file fails", ignored, "ignored"),
        ("a file not pinned against end-of-line conversion fails", eol_unpinned, "end-of-line"),
        ("a short commit id fails", edit_pin(lambda p: p.update(commit=p["commit"][:39])), "40 digit"),
        ("a pin without the CVE fix fails", edit_pin(lambda p: p.update(contains=[])), "60c0be6"),
        ("a ggml version the tree does not have fails", edit_pin(lambda p: p.update(ggml="0.0.1")), "`ggml` says"),
        ("a pin without a date fails", edit_pin(lambda p: p.pop("date")), "`date`"),
        ("a pin without upstream fails", edit_pin(lambda p: p.pop("upstream")), "`upstream`"),
        ("a pin that moved without re-vendoring fails", edit_pin(lambda p: p.update(commit="0" * 40)), "pin.json names"),
        ("a file in patches/ that is not a patch fails", stray_patch_file, "patches/"),
        ("an edited vendor/whisper.cpp/LICENSE fails", append("LICENSE", b"\n"), "byte for byte"),
        ("an upstream git settings file fails", git_settings, "git settings"),
    ]
    if hasattr(os, "symlink") and os.name != "nt":
        cases.append(("a symlink under source/ fails", symlink, "symlink"))

    failures = []
    try:
        for name, mutate, expected in cases:
            repo = sandbox / "repo"
            if repo.exists():
                shutil.rmtree(repo)
            shutil.copytree(pristine, repo, symlinks=True)
            subprocess.run(["git", "init", "-q", str(repo)], env=env, check=True)
            mutate(repo)
            run = subprocess.run(["bash", str(repo / SCRIPT_REL)], env=env, capture_output=True, text=True)
            output = run.stdout + run.stderr
            if expected is None:
                good = run.returncode == 0
            else:
                good = run.returncode != 0 and expected in output
            print(f"{'ok  ' if good else 'FAIL'} {name}")
            if not good:
                failures.append((name, run.returncode, output))
    finally:
        shutil.rmtree(sandbox, ignore_errors=True)
    for name, code, output in failures:
        print(f"\n--- {name} (exit {code}) ---\n{output}", file=sys.stderr)
    if failures:
        print(f"\n::error::{len(failures)} of {len(cases)} self test case(s) did not behave as expected", file=sys.stderr)
        return 1
    print(f"check-vendored-whisper-cpp self test: {len(cases)} cases behaved as expected")
    return 0


def main():
    parser = argparse.ArgumentParser(prog=SCRIPT_REL, description="Check the vendored whisper.cpp subset.")
    group = parser.add_mutually_exclusive_group()
    group.add_argument("--refresh", metavar="ARCHIVE_OR_CLONE", help="replace source/ with the subset at pin.json's commit")
    group.add_argument("--write-manifest", action="store_true", help="re-hash the tree already vendored")
    group.add_argument("--self-test", action="store_true", help="prove the check fails when it must")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if args.refresh:
        refresh(args.refresh)
    elif args.write_manifest:
        write_manifest()
    return report(*verify())


sys.exit(main())
PY
