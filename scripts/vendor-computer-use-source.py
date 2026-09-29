#!/usr/bin/env python3
"""Refresh `vendor/computer-use/source/` and its integrity manifest.

The included files are unmodified upstream at `pin.json`'s commit; documented
reference assets are omitted. Patches are NOT applied to the vendored files.
Keeping the two separate makes an upstream update tractable: the tree is
replaced wholesale and the reviewed patches are re-applied on top, so a
conflict is a real conflict rather than a merge of our edits with themselves.

Usage:
    python3 scripts/vendor-computer-use-source.py --from <path-to-upstream-clone>
    python3 scripts/vendor-computer-use-source.py --manifest-only
"""
import argparse
import hashlib
import json
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor/computer-use"
SOURCE = VENDOR / "source"
MANIFEST = VENDOR / "source-manifest.json"
SCHEMA = 1

# Upstream paths that are deliberately NOT vendored. The extracted assets are
# artwork from another vendor's application, outside upstream's MIT licence.
# The official bundle archives are reference inputs whose LFS objects are not
# available in this repository. None of these files is a build input. Record
# each omission in the manifest so an upstream refresh cannot restore it.
EXCLUDED = [
    (
        "docs/references/codex-computer-use-reverse-engineering/assets/extracted-",
        "assets extracted from a third party's application; upstream's MIT licence "
        "does not extend to them, and they are documentation references rather "
        "than build inputs",
    ),
    (
        ".gitattributes",
        "only configures Git LFS for the omitted official bundle archives; "
        "not a build input",
    ),
    (
        "docs/references/codex-computer-use-reverse-engineering/assets/official-bundles/",
        "official plugin archives are unavailable in this repository's Git LFS "
        "storage and are documentation references rather than build inputs",
    ),
]


def excluded_reason(relative):
    for prefix, reason in EXCLUDED:
        if relative.startswith(prefix):
            return reason
    return None


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def files():
    """Every vendored file, path-sorted, as `/`-separated repo-relative paths."""
    return sorted(
        (p for p in SOURCE.rglob("*") if p.is_file() and not p.is_symlink()),
        key=lambda p: str(p.relative_to(SOURCE)).replace("\\", "/"),
    )


def tree_digest(entries):
    """One digest over the whole tree: names AND contents.

    Hashing only the file hashes would miss a rename, and hashing only the names
    would miss an edit. The line format is fixed so the value is reproducible.
    """
    joined = "".join(f"{e['path']}\0{e['sha256']}\n" for e in entries)
    return hashlib.sha256(joined.encode()).hexdigest()


def build_manifest():
    pin = json.loads((VENDOR / "pin.json").read_text())
    entries = [
        {"path": str(p.relative_to(SOURCE)).replace("\\", "/"), "sha256": digest(p)}
        for p in files()
    ]
    return {
        "schema_version": SCHEMA,
        "upstream_repository": pin["repository"],
        "upstream_commit": pin["upstream_commit"],
        "upstream_version": pin["upstream_version"],
        "file_count": len(entries),
        "tree_sha256": tree_digest(entries),
        "excluded": [{"path_prefix": prefix, "reason": reason} for prefix, reason in EXCLUDED],
        "files": entries,
    }


def verify():
    """Raise unless the vendored tree matches the manifest exactly.

    ⚠ Checks in BOTH directions. Hashing only the files the manifest lists would
    accept an extra file nobody reviewed sitting in the build input.
    """
    if not MANIFEST.exists():
        raise ValueError(f"vendored source manifest is missing: {MANIFEST}")
    manifest = json.loads(MANIFEST.read_text())
    if manifest.get("schema_version") != SCHEMA:
        raise ValueError("unsupported vendored source manifest schema")
    pin = json.loads((VENDOR / "pin.json").read_text())
    if manifest.get("upstream_commit") != pin["upstream_commit"]:
        raise ValueError(
            "vendored source manifest names "
            f"{manifest.get('upstream_commit')} but pin.json names {pin['upstream_commit']}; "
            "re-run scripts/vendor-computer-use-source.py"
        )
    recorded = {e["path"]: e["sha256"] for e in manifest["files"]}
    present = {str(p.relative_to(SOURCE)).replace("\\", "/"): digest(p) for p in files()}
    missing = sorted(set(recorded) - set(present))
    extra = sorted(set(present) - set(recorded))
    changed = sorted(p for p in set(recorded) & set(present) if recorded[p] != present[p])
    if missing or extra or changed:
        raise ValueError(
            "vendored Open Computer Use source does not match its manifest — "
            f"missing={missing[:5]} unrecorded={extra[:5]} changed={changed[:5]}"
        )
    entries = [{"path": p, "sha256": present[p]} for p in sorted(present)]
    if tree_digest(entries) != manifest["tree_sha256"]:
        raise ValueError("vendored Open Computer Use source tree digest mismatch")
    return manifest


def prune_excluded():
    """Delete anything EXCLUDED names, and report what went."""
    removed = []
    for path in sorted(SOURCE.rglob("*")):
        if not path.is_file():
            continue
        relative = str(path.relative_to(SOURCE)).replace("\\", "/")
        reason = excluded_reason(relative)
        if reason:
            path.unlink()
            removed.append({"path": relative, "reason": reason})
    # Directories left empty by the prune would otherwise survive as noise.
    for path in sorted(SOURCE.rglob("*"), key=lambda p: -len(str(p))):
        if path.is_dir() and not any(path.iterdir()):
            path.rmdir()
    return removed


def refresh(origin):
    pin = json.loads((VENDOR / "pin.json").read_text())
    commit = pin["upstream_commit"]
    actual = subprocess.check_output(
        ["git", "rev-parse", f"{commit}^{{commit}}"], cwd=origin, text=True
    ).strip()
    if actual != commit:
        raise ValueError(f"{origin} does not contain the pinned commit {commit}")
    if SOURCE.exists():
        shutil.rmtree(SOURCE)
    SOURCE.mkdir(parents=True)
    # `git archive` exports the COMMIT's tree, so a dirty clone — one with our
    # patches already applied, which is exactly what a build leaves behind —
    # cannot leak into the vendored copy.
    archive = subprocess.check_output(["git", "archive", "--format=tar", commit], cwd=origin)
    subprocess.run(["tar", "-x", "-C", str(SOURCE)], input=archive, check=True)
    for entry in prune_excluded():
        print(f"  excluded {entry['path']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--from", dest="origin", help="an upstream clone containing the pinned commit")
    parser.add_argument("--manifest-only", action="store_true", help="re-hash the tree already vendored")
    parser.add_argument("--verify", action="store_true", help="check the tree against the manifest")
    args = parser.parse_args()
    if args.verify:
        manifest = verify()
        print(f"vendored source OK: {manifest['file_count']} files, tree {manifest['tree_sha256'][:12]}")
        return
    if args.origin:
        refresh(Path(args.origin).resolve())
    elif not args.manifest_only:
        parser.error("pass --from <clone>, --manifest-only, or --verify")
    MANIFEST.write_text(json.dumps(build_manifest(), indent=2) + "\n")
    manifest = verify()
    print(f"vendored {manifest['file_count']} files at {manifest['upstream_commit'][:12]}")
    print(f"tree_sha256 {manifest['tree_sha256']}")


if __name__ == "__main__":
    main()
