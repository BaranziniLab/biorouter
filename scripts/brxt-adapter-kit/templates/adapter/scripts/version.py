"""Move the version in every place it is written, together.

    uv run python scripts/version.py 0.3.0

It rewrites manifest.json, pyproject.toml, src/<package>/__init__.py and the
README's "Current version" line. The User-Agent the extension sends is built
from __version__, so it moves with them. It refuses a version that does not move
forward. Then run uv lock, the tests and scripts/build_brxt.py before tagging
v<version>.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VERSION = re.compile(r"\d+\.\d+\.\d+")


def bump(root: Path, version: str) -> str:
    if not VERSION.fullmatch(version):
        raise SystemExit("Use MAJOR.MINOR.PATCH, for example 0.3.0.")
    manifest_path = root / "manifest.json"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    old = manifest["version"]
    if tuple(map(int, version.split("."))) <= tuple(map(int, old.split("."))):
        raise SystemExit(f"{version} does not move forward from {old}.")
    package = manifest["name"].replace("-", "_")
    edits = [
        (root / "pyproject.toml", r'(?m)^version = "[^"]+"', f'version = "{version}"'),
        (root / "src" / package / "__init__.py", r'(?m)^__version__ = "[^"]+"', f'__version__ = "{version}"'),
        (root / "README.md", r"Current version: \*\*\d+\.\d+\.\d+\*\*", f"Current version: **{version}**"),
    ]
    for path, pattern, replacement in edits:
        text = path.read_text(encoding="utf-8")
        new, count = re.subn(pattern, replacement, text, count=1)
        if count != 1:
            raise SystemExit(f"{path.relative_to(root)} has no version line to move.")
        path.write_text(new, encoding="utf-8")
    manifest["version"] = version
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")
    return old


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("Usage: uv run python scripts/version.py X.Y.Z")
    old = bump(ROOT, sys.argv[1])
    print(f"{old} -> {sys.argv[1]}. Now run: uv lock, the tests, scripts/build_brxt.py, then tag v{sys.argv[1]}.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
