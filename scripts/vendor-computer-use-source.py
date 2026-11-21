#!/usr/bin/env python3
"""Refresh `vendor/agent-desktop/source/` and its integrity manifest.

Biorouter Copilot's engine is agent-desktop (AD-01 and AD-02 in
docs/design/copilot-agent-desktop.md). `source/` holds upstream's files at the
commit `pin.json` names, unmodified, limited to what a build reads (INCLUDED
below). There are no Biorouter patches: every Biorouter rule lives in the
wrapper in `crates/biorouter-mcp/src/computer_use/`.

Usage:
    python3 scripts/vendor-computer-use-source.py --from <path-to-upstream-clone>
    python3 scripts/vendor-computer-use-source.py --manifest-only
    python3 scripts/vendor-computer-use-source.py --dependencies
    python3 scripts/vendor-computer-use-source.py --verify

`--dependencies` rewrites `vendor/agent-desktop/DEPENDENCIES.txt`, the list of
third-party crates compiled into the helper with their licences, from `cargo
metadata`. It needs the crates in the cargo home (or the network); set
BIOROUTER_HELPER_CARGO to the cargo to run. `--verify` checks that the list
names exactly the crates `Cargo.lock` locks, so the two cannot drift.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VENDOR = ROOT / "vendor/agent-desktop"
SOURCE = VENDOR / "source"
MANIFEST = VENDOR / "source-manifest.json"
DEPENDENCIES = VENDOR / "DEPENDENCIES.txt"
SCHEMA = 2

# Upstream paths that ARE vendored, each with the reason a build or a test needs
# it. Everything else upstream (apps/, assets/, docs/, npm/, probes/, the rest of
# tests/, benchmarks/, its rust-toolchain.toml) is left out. A path ending in `/`
# is a folder; any other path is one file.
#
# ⚠ crates/ffi is included although Biorouter never builds the C library: the
# workspace's Cargo.toml lists it as a member, and cargo refuses to load a
# workspace with a member missing. Measured 2026-11-20 with the subset that left
# it out ("failed to load manifest for workspace member").
#
# ⚠ rust-toolchain.toml is left out on purpose. rustup reads it from the build
# directory, and in a clean container it silently installed upstream's 1.89.0
# instead of building with Biorouter's 1.92 (measured 2026-11-20).
INCLUDED = [
    ("Cargo.toml", "the workspace manifest"),
    ("Cargo.lock", "the exact dependency versions and their crates.io checksums"),
    ("LICENSE", "upstream's Apache License 2.0 text"),
    ("clippy.toml", "upstream's lint settings, read when its crates are linted"),
    ("deny.toml", "upstream's dependency licence and source policy"),
    ("crates/", "the core crate, the three platform adapters and the workspace member crates/ffi"),
    ("src/", "the agent-desktop binary"),
    ("skills/", "embedded with include_str! by crates/core/src/commands/skills.rs"),
    ("providers/macos/", "embedded with include_str! by crates/core/src/sandbox/provider_table.rs"),
    ("tests/fixtures/drilldown-refmap.json", "read by agent-desktop-core's unit tests with include_str!"),
    ("tests/fixtures/skeleton-tree.json", "read by agent-desktop-core's unit tests with include_str!"),
    ("tests/fixtures/trace_show/", "read by agent-desktop-core's unit tests"),
]

# Crates that would give the helper a network client. agent-desktop has none
# (AD-20); a pin move that brings one in must be a reviewed decision, so the
# check fails rather than letting it ride along.
NETWORK_CRATES = {
    "reqwest", "hyper", "hyper-util", "ureq", "isahc", "curl", "curl-sys", "surf",
    "attohttpc", "minreq", "ehttp", "awc", "h2", "h3", "quinn", "rustls",
    "native-tls", "openssl", "openssl-sys", "tungstenite", "tokio-tungstenite",
}


def included_reason(relative):
    for path, reason in INCLUDED:
        if relative == path or (path.endswith("/") and relative.startswith(path)):
            return reason
    return None


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def relative(path):
    return str(path.relative_to(SOURCE)).replace("\\", "/")


def files():
    """Every vendored file, path-sorted, as `/`-separated source-relative paths."""
    return sorted((p for p in SOURCE.rglob("*") if p.is_file() and not p.is_symlink()), key=relative)


def tree_digest(entries):
    """One digest over the whole tree: names AND contents.

    Hashing only the file hashes would miss a rename, and hashing only the names
    would miss an edit. The line format is fixed so the value is reproducible.
    """
    joined = "".join(f"{e['path']}\0{e['sha256']}\n" for e in entries)
    return hashlib.sha256(joined.encode()).hexdigest()


def pin():
    return json.loads((VENDOR / "pin.json").read_text(encoding="utf-8"))


def locked_packages(lock_text):
    """Every package name in a Cargo.lock, in order."""
    return re.findall(r'^name = "([^"]+)"$', lock_text, flags=re.MULTILINE)


def network_crates(lock_text):
    return sorted(set(locked_packages(lock_text)) & NETWORK_CRATES)


def lock_entries(lock_text):
    """(name, version) of every crates.io package in a Cargo.lock."""
    entries = set()
    for block in lock_text.split("[[package]]")[1:]:
        name = re.search(r'^name = "([^"]+)"$', block, flags=re.MULTILINE)
        version = re.search(r'^version = "([^"]+)"$', block, flags=re.MULTILINE)
        if name and version and 'source = "registry+' in block:
            entries.add((name.group(1), version.group(1)))
    return entries


DEPENDENCIES_HEADER = """Third-party Rust crates in Biorouter Copilot's engine
=====================================================

Biorouter Copilot runs agent-desktop {version} (Vercel Labs, Apache License 2.0),
built from the source vendored at vendor/agent-desktop/source. These crates.io
crates are compiled into it. Each crate's source, which carries its licence text,
is published at https://crates.io/crates/<crate>/<version>, and the exact versions
and checksums are in vendor/agent-desktop/source/Cargo.lock.

The platforms column names the targets whose binary includes the crate (normal and
build dependencies; development dependencies are not compiled into the helper).
Crates no target compiles (the lockfile locks them for other platforms, or only
for tests) are listed with "-".

Licences, counted over the crates below:
{families}

"""


def dependency_rows(cargo):
    """One row per locked crates.io package: name, version, licence, targets that compile it."""
    current = pin()
    manifest_path = SOURCE / "Cargo.toml"
    rows, licenses = {}, {}
    for target, triple in current["targets"].items():
        result = subprocess.run(
            [cargo, "metadata", "--locked", "--format-version", "1", "--filter-platform", triple,
             "--manifest-path", str(manifest_path)],
            check=True, capture_output=True, text=True,
        )
        metadata = json.loads(result.stdout)
        packages = {package["id"]: package for package in metadata["packages"]}
        for package in packages.values():
            licenses[(package["name"], package["version"])] = package.get("license") or "(none declared)"
        nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
        roots = [pid for pid, package in packages.items()
                 if package["name"] == "agent-desktop" and package.get("source") is None]
        if len(roots) != 1:
            raise ValueError(f"cannot find the agent-desktop binary package for {triple}")
        seen, stack = set(), roots
        while stack:
            node = nodes[stack.pop()]
            if node["id"] in seen:
                continue
            seen.add(node["id"])
            for dep in node["deps"]:
                if any(kind["kind"] in (None, "build") for kind in dep["dep_kinds"]):
                    stack.append(dep["pkg"])
        for pid in seen:
            package = packages[pid]
            if package.get("source") is None:
                continue
            key = (package["name"], package["version"])
            row = rows.setdefault(key, {"license": package.get("license") or "(none declared)", "targets": []})
            row["targets"].append(target)
    # `--filter-platform` drops the packages no target reaches, so read their
    # licences from one unfiltered pass.
    everything = json.loads(subprocess.run(
        [cargo, "metadata", "--locked", "--format-version", "1", "--manifest-path", str(manifest_path)],
        check=True, capture_output=True, text=True,
    ).stdout)
    for package in everything["packages"]:
        licenses[(package["name"], package["version"])] = package.get("license") or "(none declared)"
    for key in lock_entries((SOURCE / "Cargo.lock").read_text(encoding="utf-8")) - set(rows):
        rows[key] = {"license": licenses.get(key, "(none declared)"), "targets": []}
    return rows


def render_dependencies(rows):
    families = {}
    for row in rows.values():
        families[row["license"]] = families.get(row["license"], 0) + 1
    family_lines = "\n".join(f"  {count:3d}  {name}" for name, count in
                             sorted(families.items(), key=lambda item: (-item[1], item[0])))
    lines = [DEPENDENCIES_HEADER.format(version=pin()["upstream_version"], families=family_lines)]
    width = max(len(name) for name, _ in rows) + 2
    vwidth = max(len(version) for _, version in rows) + 2
    lwidth = max(len(row["license"]) for row in rows.values()) + 2
    lines.append(f"{'crate':<{width}}{'version':<{vwidth}}{'licence':<{lwidth}}platforms")
    for (name, version), row in sorted(rows.items()):
        targets = ",".join(sorted(row["targets"])) or "-"
        lines.append(f"{name:<{width}}{version:<{vwidth}}{row['license']:<{lwidth}}{targets}".rstrip())
    return "\n".join(lines) + "\n"


def listed_dependencies(text):
    """(name, version) of every row in DEPENDENCIES.txt."""
    table = text.split("\ncrate ", 1)
    if len(table) != 2:
        raise ValueError("vendor/agent-desktop/DEPENDENCIES.txt has no crate table")
    rows = set()
    for line in table[1].splitlines()[1:]:
        parts = line.split()
        if len(parts) >= 2:
            rows.add((parts[0], parts[1]))
    return rows


def build_manifest():
    current = pin()
    entries = [{"path": relative(p), "sha256": digest(p)} for p in files()]
    return {
        "schema_version": SCHEMA,
        "engine": current["engine"],
        "upstream_repository": current["repository"],
        "upstream_tag": current["upstream_tag"],
        "upstream_commit": current["upstream_commit"],
        "upstream_version": current["upstream_version"],
        "file_count": len(entries),
        "tree_sha256": tree_digest(entries),
        "included": [{"path": path, "reason": reason} for path, reason in INCLUDED],
        "files": entries,
    }


def verify():
    """Raise unless the vendored tree matches the manifest exactly.

    ⚠ Checks in BOTH directions. Hashing only the files the manifest lists would
    accept an extra file nobody reviewed sitting in the build input.
    """
    if not MANIFEST.exists():
        raise ValueError(f"vendored source manifest is missing: {MANIFEST}")
    manifest = json.loads(MANIFEST.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != SCHEMA:
        raise ValueError("unsupported vendored source manifest schema")
    current = pin()
    for key in ("engine", "upstream_tag", "upstream_commit", "upstream_version"):
        if manifest.get(key) != current[key]:
            raise ValueError(
                f"vendored source manifest names {key} {manifest.get(key)!r} but pin.json names "
                f"{current[key]!r}; re-run scripts/vendor-computer-use-source.py"
            )
    if manifest.get("upstream_repository") != current["repository"]:
        raise ValueError("vendored source manifest and pin.json name different repositories")
    recorded = {e["path"]: e["sha256"] for e in manifest["files"]}
    present = {relative(p): digest(p) for p in files()}
    missing = sorted(set(recorded) - set(present))
    extra = sorted(set(present) - set(recorded))
    changed = sorted(p for p in set(recorded) & set(present) if recorded[p] != present[p])
    if missing or extra or changed:
        raise ValueError(
            "vendored agent-desktop source does not match its manifest: "
            f"missing={missing[:5]} unrecorded={extra[:5]} changed={changed[:5]}"
        )
    entries = [{"path": p, "sha256": present[p]} for p in sorted(present)]
    if tree_digest(entries) != manifest["tree_sha256"]:
        raise ValueError("vendored agent-desktop source tree digest mismatch")
    if manifest.get("file_count") != len(entries):
        raise ValueError("vendored agent-desktop source file count does not match its manifest")
    outside = sorted(p for p in present if included_reason(p) is None)
    if outside:
        raise ValueError(f"vendored agent-desktop source holds files outside the included subset: {outside[:5]}")
    declared = [(entry["path"], entry["reason"]) for entry in manifest.get("included", [])]
    if declared != INCLUDED:
        raise ValueError("the manifest's included subset differs from the vendoring script's; re-run it")
    if (VENDOR / "LICENSE").read_bytes() != (SOURCE / "LICENSE").read_bytes():
        raise ValueError("vendor/agent-desktop/LICENSE differs from upstream's source/LICENSE")
    lock_text = (SOURCE / "Cargo.lock").read_text(encoding="utf-8")
    if not DEPENDENCIES.exists():
        raise ValueError("vendor/agent-desktop/DEPENDENCIES.txt is missing; run with --dependencies")
    listed = listed_dependencies(DEPENDENCIES.read_text(encoding="utf-8"))
    locked = lock_entries(lock_text)
    if listed != locked:
        raise ValueError(
            "vendor/agent-desktop/DEPENDENCIES.txt does not list exactly the crates Cargo.lock locks: "
            f"unlisted={sorted(locked - listed)[:5]} stale={sorted(listed - locked)[:5]}; "
            "re-run scripts/vendor-computer-use-source.py --dependencies"
        )
    network = network_crates(lock_text)
    if network:
        raise ValueError(
            f"agent-desktop's Cargo.lock now locks network client crates {network}; Biorouter Copilot's "
            "helper must stay without one (AD-20). Review the pin move before vendoring it."
        )
    return manifest


def refresh(origin):
    current = pin()
    commit = current["upstream_commit"]
    actual = subprocess.check_output(["git", "rev-parse", f"{commit}^{{commit}}"], cwd=origin, text=True).strip()
    if actual != commit:
        raise ValueError(f"{origin} does not contain the pinned commit {commit}")
    tag = subprocess.run(["git", "rev-parse", f"{current['upstream_tag']}^{{commit}}"], cwd=origin,
                         capture_output=True, text=True)
    if tag.returncode == 0 and tag.stdout.strip() != commit:
        raise ValueError(f"tag {current['upstream_tag']} in {origin} is {tag.stdout.strip()}, not the pinned {commit}")
    if SOURCE.exists():
        shutil.rmtree(SOURCE)
    SOURCE.mkdir(parents=True)
    # `git archive` exports the COMMIT's tree, so a dirty clone (one a build or an
    # experiment has written into) cannot leak into the vendored copy.
    paths = [path.rstrip("/") for path, _ in INCLUDED]
    archive = subprocess.check_output(["git", "archive", "--format=tar", commit, "--", *paths], cwd=origin)
    subprocess.run(["tar", "-x", "-C", str(SOURCE)], input=archive, check=True)
    shutil.copy2(SOURCE / "LICENSE", VENDOR / "LICENSE")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--from", dest="origin", help="an upstream clone containing the pinned commit")
    parser.add_argument("--manifest-only", action="store_true", help="re-hash the tree already vendored")
    parser.add_argument("--verify", action="store_true", help="check the tree against the manifest")
    parser.add_argument("--dependencies", action="store_true",
                        help="regenerate DEPENDENCIES.txt from cargo metadata (needs the crates)")
    args = parser.parse_args()
    if args.dependencies:
        cargo = os.environ.get("BIOROUTER_HELPER_CARGO") or "cargo"
        DEPENDENCIES.write_text(render_dependencies(dependency_rows(cargo)), encoding="utf-8")
        print(f"wrote {DEPENDENCIES.relative_to(ROOT)}")
        return
    if args.verify:
        manifest = verify()
        print(f"vendored agent-desktop source OK: {manifest['file_count']} files, tree {manifest['tree_sha256'][:12]}")
        return
    if args.origin:
        refresh(Path(args.origin).resolve())
    elif not args.manifest_only:
        parser.error("pass --from <clone>, --manifest-only, --dependencies, or --verify")
    MANIFEST.write_text(json.dumps(build_manifest(), indent=2) + "\n", encoding="utf-8")
    manifest = verify()
    print(f"vendored {manifest['file_count']} files at {manifest['upstream_commit'][:12]}")
    print(f"tree_sha256 {manifest['tree_sha256']}")


if __name__ == "__main__":
    main()
