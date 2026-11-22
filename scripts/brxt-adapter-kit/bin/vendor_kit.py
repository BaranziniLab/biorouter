"""Copy the kit into an adapter repository, or check that its copy is unchanged.

    python bin/vendor_kit.py ADAPTER_DIR            # copy the kit in
    python bin/vendor_kit.py ADAPTER_DIR --check    # exit 1 if the copy differs from this kit

The runtime half (brxt_kit/) goes to src/<package>/_kit/ and the testing half
(brxt_kit_testing/) to tests/kit_testing/. Both are copied byte for byte, and
src/<package>/_kit/vendored.json records the kit version and the SHA-256 of every
copied file. The adapter's contract test and build script refuse any difference,
so a vendored file can only change by vendoring a reviewed kit again.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

KIT_ROOT = Path(__file__).resolve().parents[1]
RUNTIME = KIT_ROOT / "brxt_kit"
TESTING = KIT_ROOT / "brxt_kit_testing"


def kit_version() -> str:
    namespace: dict[str, str] = {}
    exec(compile((RUNTIME / "__init__.py").read_text(encoding="utf-8"), "brxt_kit/__init__.py", "exec"), namespace)
    return namespace["KIT_VERSION"]


def package_of(adapter: Path) -> str:
    manifest = json.loads((adapter / "manifest.json").read_text(encoding="utf-8"))
    return manifest["name"].replace("-", "_")


def planned_files(adapter: Path) -> dict[str, Path]:
    package = package_of(adapter)
    files = {f"src/{package}/_kit/{p.name}": p for p in sorted(RUNTIME.glob("*.py"))}
    files.update({f"tests/kit_testing/{p.name}": p for p in sorted(TESTING.glob("*.py"))})
    return files


def vendor(adapter: Path) -> dict[str, str]:
    package = package_of(adapter)
    files = planned_files(adapter)
    for directory in (adapter / "src" / package / "_kit", adapter / "tests" / "kit_testing"):
        directory.mkdir(parents=True, exist_ok=True)
        for stale in directory.glob("*.py"):
            if stale.relative_to(adapter).as_posix() not in files:
                stale.unlink()
    record: dict[str, str] = {}
    for relative, source in files.items():
        data = source.read_bytes()
        (adapter / relative).write_bytes(data)
        record[relative] = hashlib.sha256(data).hexdigest()
    (adapter / "src" / package / "_kit" / "vendored.json").write_text(
        json.dumps({"kit_version": kit_version(), "files": dict(sorted(record.items()))}, indent=2) + "\n",
        encoding="utf-8",
    )
    return record


def check(adapter: Path) -> list[str]:
    problems: list[str] = []
    package = package_of(adapter)
    record_path = adapter / "src" / package / "_kit" / "vendored.json"
    if not record_path.is_file():
        return [f"{record_path.relative_to(adapter)} is missing"]
    record = json.loads(record_path.read_text(encoding="utf-8"))
    if record.get("kit_version") != kit_version():
        problems.append(f"vendored kit {record.get('kit_version')} is not this kit ({kit_version()})")
    files = planned_files(adapter)
    if set(record.get("files") or {}) != set(files):
        problems.append("vendored.json lists other files than this kit has")
    for relative, source in files.items():
        target = adapter / relative
        if not target.is_file():
            problems.append(f"{relative} is missing")
        elif target.read_bytes() != source.read_bytes():
            problems.append(f"{relative} differs from the kit")
    return problems


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("adapter", type=Path)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    adapter = args.adapter.resolve()
    if args.check:
        problems = check(adapter)
        for problem in problems:
            print(f"vendor_kit: {problem}", file=sys.stderr)
        return 1 if problems else 0
    record = vendor(adapter)
    print(f"Vendored brxt-adapter-kit {kit_version()} into {adapter} ({len(record)} files).")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
