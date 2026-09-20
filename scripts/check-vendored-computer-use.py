#!/usr/bin/env python3
"""Assert the vendored Computer Use tree can actually survive a fresh checkout.

`vendor-computer-use-source.py --verify` proves the tree on THIS disk matches its
manifest. That is necessary and not sufficient: a tree can verify here and still
arrive different, or not at all, on someone else's machine. Two ways, both
measured on CI rather than imagined.
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = ROOT / "third_party/open-computer-use/source-manifest.json"
PREFIX = "third_party/open-computer-use/source"


def paths():
    manifest = json.loads(MANIFEST.read_text())
    return [f"{PREFIX}/{entry['path']}" for entry in manifest["files"]]


def git(args, payload):
    return subprocess.run(
        ["git", *args], input="\n".join(payload), capture_output=True, text=True, cwd=ROOT
    ).stdout.splitlines()


def main():
    wanted = paths()
    problems = []

    # 1. Ignored files never reach a fresh clone. The manifest would then fail
    #    there naming the files, which says nothing about the reason. Measured:
    #    repository-wide `*.png`, `.agents/` and `.mcp.json` rules matched 15 of
    #    them when this tree was first vendored.
    ignored = [line for line in git(["check-ignore", "--stdin"], wanted) if line.strip()]
    if ignored:
        problems.append(
            "gitignored, so they would never reach a fresh clone "
            "(add a negation to .gitignore; see its vendored block):\n  "
            + "\n  ".join(ignored[:20])
        )

    # 2. End-of-line conversion changes bytes nobody edited. The tree is HASHED,
    #    so a Windows checkout rewriting LF to CRLF fails the manifest on a clean
    #    tree. Measured on CI: five extensionless files — .gitattributes,
    #    .gitignore, CODEOWNERS, LICENSE, Makefile — matched no `text eol=lf`
    #    rule and were rewritten.
    #
    #    ⚠ Checked on EVERY platform, because whoever breaks it will be on macOS
    #    or Linux and will see nothing.
    attrs = git(["check-attr", "--stdin", "text"], wanted)
    unpinned = [line for line in attrs if line.strip() and not line.endswith(": text: unset")]
    if unpinned:
        problems.append(
            "not exempt from end-of-line conversion, so a Windows checkout would "
            "rewrite them (see the vendored block in .gitattributes):\n  "
            + "\n  ".join(unpinned[:20])
        )

    if problems:
        print(f"{len(problems)} problem(s) with the vendored Computer Use tree:\n", file=sys.stderr)
        for problem in problems:
            print(f"- {len(wanted)} vendored files checked; some are {problem}\n", file=sys.stderr)
        raise SystemExit(1)

    print(f"Vendored Computer Use source: {len(wanted)} files, committable and EOL-pinned")


if __name__ == "__main__":
    main()
