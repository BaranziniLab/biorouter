#!/usr/bin/env python3
"""Every workflow runs with the least token it needs, and every action is pinned.

Usage:
  python3 -I scripts/check-workflow-hardening.py            check the tracked tree
  python3 -I scripts/check-workflow-hardening.py --self-test

This repository's default workflow token is write on every scope. A workflow with no
`permissions:` block hands that token to every job it runs, pull requests from branches
of this repository included, and an action named by a tag or a branch runs whatever the
tag or branch points at on the day the job starts. Both were true here until J-F11
(B3 defect 18): four workflows had no block, and every action was named by a moving
ref. The rules, each checked by line so no YAML library is needed:

1. Every workflow under .github/workflows has a top-level `permissions:` block.
2. That top-level block grants no write scope. A job that writes (a release, Pages, a
   deployment) asks for the scope in its own `permissions:` block, so the grant reaches
   that job and no other.
3. No `permissions: write-all` anywhere.
4. Every `uses:` in a workflow, in a composite action under .github/actions, and in a
   workflow template of the BRXT adapter kit (scripts/brxt-adapter-kit/templates) names
   a local path (`./...`), a container by digest (`docker://...@sha256:...`), or an
   action at a full 40-character commit SHA followed by `# vX.Y.Z`, the release that
   commit carries. An action that publishes no releases is listed in UNTAGGED with the
   branch it follows, and its comment names that branch.

To move a pin, resolve the new release's commit with `gh api` (a tag can point at a tag
object; follow it to the commit) and write both the SHA and the version; never write a
SHA from memory.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# Actions that publish no release tags. The comment after the SHA names the branch.
UNTAGGED = {
    "dtolnay/rust-toolchain": "master",
}

USES = re.compile(
    r"""^\s*(?:-\s+)?uses:\s*(?P<quote>['"]?)(?P<target>[^'"\s#]+)(?P=quote)\s*(?:\#\s*(?P<comment>.*?))?\s*$"""
)
ACTION = re.compile(r"^(?P<action>[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)(?P<path>/[^@]*)?@(?P<ref>.+)$")
FULL_SHA = re.compile(r"^[0-9a-f]{40}$")
RELEASE = re.compile(r"^v\d+\.\d+\.\d+(?:\s.*)?$")
DOCKER_DIGEST = re.compile(r"^docker://[^@\s]+@sha256:[0-9a-f]{64}$")
TOP_LEVEL_KEY = re.compile(r"^[A-Za-z_][\w-]*\s*:")
SCOPE = re.compile(r"^\s+(?P<scope>[a-z][a-z-]*)\s*:\s*(?P<level>[a-z-]+)\s*(?:#.*)?$")


def _strip_comment(value: str) -> str:
    return value.split("#", 1)[0].strip()


def workflow_files(root: Path) -> list[Path]:
    folder = root / ".github" / "workflows"
    return sorted(p for p in folder.glob("*") if p.suffix in {".yml", ".yaml"})


def action_files(root: Path) -> list[Path]:
    folder = root / ".github" / "actions"
    return sorted(p for p in folder.glob("**/action.y*ml") if p.suffix in {".yml", ".yaml"})


def template_files(root: Path) -> list[Path]:
    folder = root / "scripts" / "brxt-adapter-kit" / "templates"
    return sorted(p for p in folder.glob("**/.github/workflows/*") if p.is_file())


def check_permissions(rel: str, lines: list[str], out: list[str]) -> None:
    """Rules 1 to 3 for one workflow file."""
    start = None
    for number, line in enumerate(lines):
        if re.match(r"^permissions\s*:", line):
            start = number
            break
    if start is None:
        out.append(f"{rel}: no top-level permissions block; its jobs get the repository's default token, which is write")
    else:
        inline = _strip_comment(lines[start].split(":", 1)[1])
        grants: list[tuple[int, str, str]] = []
        if inline:
            if inline == "write-all":
                grants.append((start, "*", "write"))
            elif inline.startswith("{"):
                for pair in inline.strip("{} ").split(","):
                    if ":" in pair:
                        scope, level = (part.strip() for part in pair.split(":", 1))
                        grants.append((start, scope, level))
            elif inline not in {"read-all"}:
                out.append(f"{rel}:{start + 1}: unrecognised top-level permissions value {inline!r}")
        else:
            for number in range(start + 1, len(lines)):
                line = lines[number]
                if not line.strip() or line.lstrip().startswith("#"):
                    continue
                if not line.startswith((" ", "\t")):
                    break
                match = SCOPE.match(line)
                if not match:
                    out.append(f"{rel}:{number + 1}: unrecognised line in the top-level permissions block")
                    continue
                grants.append((number, match["scope"], match["level"]))
        for number, scope, level in grants:
            if level == "write":
                out.append(
                    f"{rel}:{number + 1}: the top-level permissions block grants {scope}: write;"
                    " grant it in the permissions block of the job that writes"
                )
    for number, line in enumerate(lines):
        if re.match(r"^\s*permissions\s*:\s*write-all\s*(?:#.*)?$", line):
            out.append(f"{rel}:{number + 1}: permissions: write-all; name the scopes the job needs")


def check_uses(rel: str, lines: list[str], out: list[str]) -> int:
    """Rule 4 for one file. Returns the number of `uses:` lines read."""
    seen = 0
    for number, line in enumerate(lines):
        if not re.match(r"^\s*(?:-\s+)?uses\s*:", line):
            continue
        seen += 1
        where = f"{rel}:{number + 1}"
        match = USES.match(line)
        if not match:
            out.append(f"{where}: cannot read this uses line")
            continue
        target, comment = match["target"], (match["comment"] or "").strip()
        if target.startswith("./"):
            continue
        if target.startswith("docker://"):
            if not DOCKER_DIGEST.match(target):
                out.append(f"{where}: {target} names a container by tag; name it by @sha256 digest")
            continue
        action = ACTION.match(target)
        if not action:
            out.append(f"{where}: {target} is not owner/repo@ref")
            continue
        ref = action["ref"]
        if not FULL_SHA.match(ref):
            out.append(f"{where}: {target} is on the moving ref {ref!r}; pin it to the full commit SHA")
            continue
        branch = UNTAGGED.get(action["action"])
        if branch is not None:
            if not (comment == branch or comment.startswith(branch + " ")):
                out.append(f"{where}: {action['action']} publishes no releases; the comment after the SHA must name the branch {branch!r}")
        elif not RELEASE.match(comment):
            out.append(f"{where}: {target} has no '# vX.Y.Z' comment naming the release the commit carries")
    return seen


def run(root: Path) -> tuple[list[str], int, int]:
    out: list[str] = []
    files = 0
    uses = 0
    for path in workflow_files(root):
        rel = str(path.relative_to(root))
        lines = path.read_text(encoding="utf-8").splitlines()
        check_permissions(rel, lines, out)
        uses += check_uses(rel, lines, out)
        files += 1
    for path in action_files(root) + template_files(root):
        rel = str(path.relative_to(root))
        uses += check_uses(rel, path.read_text(encoding="utf-8").splitlines(), out)
        files += 1
    return out, files, uses


def main() -> int:
    problems, files, uses = run(ROOT)
    if not files:
        print("::error::check-workflow-hardening found no workflow files; is the root right?")
        return 1
    for problem in problems:
        print(f"::error::{problem}")
    if problems:
        print(f"check-workflow-hardening: {len(problems)} problem(s) in {files} file(s)")
        return 1
    print(f"check-workflow-hardening: {files} file(s), {uses} uses line(s) pinned or local, every workflow least privilege")
    return 0


SHA = "0123456789abcdef0123456789abcdef01234567"
GOOD_WORKFLOW = f"""name: Good
on: push
permissions:
  contents: read
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@{SHA} # v4.4.0
      - uses: "actions/cache/restore@{SHA}"  # v4.3.0
      - uses: dtolnay/rust-toolchain@{SHA} # master
      - uses: ./.github/actions/local
      - uses: docker://alpine@sha256:{"a" * 64}
  publish:
    permissions:
      contents: write
    runs-on: ubuntu-latest
    steps:
      - run: echo publish
"""


def self_test() -> int:
    cases = [
        ("a good workflow", GOOD_WORKFLOW, None),
        ("no top-level permissions", GOOD_WORKFLOW.replace("permissions:\n  contents: read\n", ""), "no top-level permissions block"),
        ("a top-level write scope", GOOD_WORKFLOW.replace("  contents: read\n", "  contents: read\n  pages: write\n", 1), "grants pages: write"),
        ("an inline top-level write", GOOD_WORKFLOW.replace("permissions:\n  contents: read\n", "permissions: { contents: write }\n"), "grants contents: write"),
        ("top-level write-all", GOOD_WORKFLOW.replace("permissions:\n  contents: read\n", "permissions: write-all\n"), "grants *: write"),
        ("job-level write-all", GOOD_WORKFLOW.replace("    permissions:\n      contents: write\n", "    permissions: write-all\n"), "permissions: write-all"),
        ("a major-version tag", GOOD_WORKFLOW.replace(f"actions/checkout@{SHA} # v4.4.0", "actions/checkout@v4"), "moving ref 'v4'"),
        ("a branch", GOOD_WORKFLOW.replace(f"dtolnay/rust-toolchain@{SHA} # master", "dtolnay/rust-toolchain@master"), "moving ref 'master'"),
        ("a short SHA", GOOD_WORKFLOW.replace(f"actions/checkout@{SHA}", f"actions/checkout@{SHA[:12]}"), "moving ref"),
        ("no version comment", GOOD_WORKFLOW.replace(f"actions/checkout@{SHA} # v4.4.0", f"actions/checkout@{SHA}"), "no '# vX.Y.Z' comment"),
        ("a major-only comment", GOOD_WORKFLOW.replace("# v4.4.0", "# v4"), "no '# vX.Y.Z' comment"),
        ("an untagged action without its branch", GOOD_WORKFLOW.replace("# master", "# v1.0.0"), "must name the branch 'master'"),
        ("a container by tag", GOOD_WORKFLOW.replace(f"docker://alpine@sha256:{'a' * 64}", "docker://alpine:3"), "by tag"),
    ]
    failures = 0
    for label, text, expected in cases:
        with tempfile.TemporaryDirectory(prefix="workflow-hardening-") as tmp:
            root = Path(tmp)
            (root / ".github" / "workflows").mkdir(parents=True)
            (root / ".github" / "workflows" / "case.yml").write_text(text, encoding="utf-8")
            problems, _, _ = run(root)
        ok = (not problems) if expected is None else any(expected in p for p in problems)
        if not ok:
            failures += 1
            print(f"::error::self test case {label!r}: expected {expected!r}, got {problems}")

    # A composite action and a kit template are read for rule 4, not for rules 1 to 3.
    with tempfile.TemporaryDirectory(prefix="workflow-hardening-") as tmp:
        root = Path(tmp)
        (root / ".github" / "workflows").mkdir(parents=True)
        (root / ".github" / "workflows" / "case.yml").write_text(GOOD_WORKFLOW, encoding="utf-8")
        action = root / ".github" / "actions" / "thing" / "action.yaml"
        action.parent.mkdir(parents=True)
        action.write_text("runs:\n  using: composite\n  steps:\n    - uses: actions/checkout@v4\n", encoding="utf-8")
        template = root / "scripts" / "brxt-adapter-kit" / "templates" / "adapter" / ".github" / "workflows" / "ci.yml.tmpl"
        template.parent.mkdir(parents=True)
        template.write_text("permissions:\n  contents: write\njobs:\n  t:\n    steps:\n      - uses: astral-sh/setup-uv@v6\n", encoding="utf-8")
        problems, files, _ = run(root)
        expected = ["action.yaml:4: actions/checkout@v4", "ci.yml.tmpl:6: astral-sh/setup-uv@v6"]
        if files != 3 or not all(any(e in p for p in problems) for e in expected) or any("ci.yml.tmpl" in p and "write" in p for p in problems):
            failures += 1
            print(f"::error::self test case 'composite action and kit template': got {files} file(s), {problems}")

    if failures:
        print(f"::error::{failures} check-workflow-hardening self test case(s) did not behave as expected")
        return 1
    print(f"check-workflow-hardening self test: {len(cases) + 1} cases behaved as expected")
    return 0


if __name__ == "__main__":
    arguments = sys.argv[1:]
    if arguments == ["--self-test"]:
        sys.exit(self_test())
    if arguments:
        print("usage: check-workflow-hardening.py [--self-test]", file=sys.stderr)
        sys.exit(2)
    sys.exit(main())
