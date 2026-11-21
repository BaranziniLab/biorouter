#!/usr/bin/env python3
"""Report what has changed upstream since the vendored agent-desktop release.

Read-only. It never writes to the tree, never changes the pin, and never applies
anything: deciding to take an upstream release is a review, not a command.

    python3 scripts/check-computer-use-upstream.py              # against the latest release
    python3 scripts/check-computer-use-upstream.py --tag v1.4.0 # against one tag
    python3 scripts/check-computer-use-upstream.py --json

Biorouter vendors agent-desktop unmodified (AD-02 in
docs/design/copilot-agent-desktop.md), so there are no patches to conflict. What a
pin move costs is in three places, and this report names each:

  * files inside the vendored subset that changed (the build input);
  * `Cargo.lock`, because a dependency change means re-reading licences
    (`vendor-computer-use-source.py --dependencies`) and the network-crate check;
  * the JSON contract and command line, because the wrapper in
    crates/biorouter-mcp/src/computer_use/ builds every command from an allowlist:
    changes under src/cli*, crates/core/src/commands and the json-output docs are
    listed so they can be read before the wrapper's contract tests run.

The procedure for moving the pin is in vendor/agent-desktop/README.md.
"""
import argparse
import importlib.util
import json
import re
import subprocess
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "vendor/agent-desktop"
API = "https://api.github.com"

_spec = importlib.util.spec_from_file_location("vendor_source", Path(__file__).with_name("vendor-computer-use-source.py"))
vendor_source = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(vendor_source)

CONTRACT_PATHS = ("src/cli", "src/cli_args", "src/dispatch", "crates/core/src/commands",
                  "docs/json-output.md", "apps/docs/content/docs/json-output.mdx",
                  "apps/docs/content/docs/errors-and-recovery.mdx")


def owner_repo(url):
    match = re.search(r"github\.com[/:]([^/]+)/([^/.]+)", url)
    if not match:
        raise ValueError(f"cannot read an owner/repo out of {url!r}")
    return match.group(1), match.group(2)


def fetch(url):
    request = urllib.request.Request(url, headers={
        "Accept": "application/vnd.github+json",
        "User-Agent": "biorouter-copilot-upstream-check",
    })
    # A token lifts the 60/hour anonymous rate limit; it is optional and never required.
    token = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True)
    if token.returncode == 0 and token.stdout.strip():
        request.add_header("Authorization", f"Bearer {token.stdout.strip()}")
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def area(path):
    parts = path.split("/")
    if parts[0] == "crates" and len(parts) > 1:
        return "crates/" + parts[1]
    return parts[0] if len(parts) > 1 else path


def classify(changed):
    vendored = sorted(path for path in changed if vendor_source.included_reason(path))
    areas = {}
    for path in vendored:
        areas[area(path)] = areas.get(area(path), 0) + 1
    contract = sorted(path for path in changed if path.startswith(CONTRACT_PATHS))
    return vendored, areas, contract


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--json", action="store_true", help="emit the report as JSON")
    parser.add_argument("--tag", help="compare with this tag instead of the latest release")
    parser.add_argument("--limit", type=int, default=30, help="how many commits to list")
    args = parser.parse_args()

    pin = json.loads((VENDOR / "pin.json").read_text(encoding="utf-8"))
    owner, repo = owner_repo(pin["repository"])
    pinned = pin["upstream_commit"]
    try:
        tag = args.tag or fetch(f"{API}/repos/{owner}/{repo}/releases/latest")["tag_name"]
        compare = fetch(f"{API}/repos/{owner}/{repo}/compare/{pinned}...{tag}")
    except urllib.error.HTTPError as error:
        raise SystemExit(
            f"upstream check failed: {error.code} {error.reason}. This is only a convenience: the "
            "vendored source in this repository is what builds, and it needs no network."
        ) from None
    except urllib.error.URLError as error:
        raise SystemExit(f"upstream check could not reach GitHub: {error.reason}") from None

    commits = compare.get("commits", [])
    changed = {entry["filename"] for entry in compare.get("files", [])}
    vendored, areas, contract = classify(changed)
    report = {
        "repository": pin["repository"],
        "vendored_tag": pin["upstream_tag"],
        "vendored_commit": pinned,
        "compared_with": tag,
        "upstream_commit": commits[-1]["sha"] if commits else pinned,
        "ahead_by": compare.get("ahead_by", len(commits)),
        "files_changed_upstream": len(changed),
        "files_changed_in_vendored_subset": len(vendored),
        "vendored_areas": areas,
        "cargo_lock_changed": "Cargo.lock" in changed,
        "contract_files_changed": contract,
        "truncated_file_list": len(compare.get("files", [])) >= 300,
        "commits": [{"sha": c["sha"][:12], "subject": (c["commit"]["message"].splitlines() or [""])[0]}
                    for c in commits[-args.limit:]],
    }
    if args.json:
        print(json.dumps(report, indent=2))
        return
    print(f"vendored : {pin['upstream_tag']} ({pinned[:12]})")
    print(f"compared : {tag} ({report['upstream_commit'][:12]})")
    if not commits:
        print("\nUp to date: nothing upstream between the vendored release and that tag.")
        return
    print(f"\n{len(commits)} commit(s), {len(changed)} file(s) changed, {len(vendored)} inside the vendored subset:")
    for name, count in sorted(areas.items()):
        print(f"  {count:5d}  {name}")
    if report["truncated_file_list"]:
        print("  (GitHub lists at most 300 files per comparison; clone both tags to see every change)")
    print()
    if report["cargo_lock_changed"]:
        print("Cargo.lock changed: regenerate DEPENDENCIES.txt and re-read the licences and the")
        print("network-crate check before vendoring.")
    if contract:
        print(f"{len(contract)} file(s) on the command line or JSON contract changed; read them before")
        print("running the wrapper's contract tests:")
        for path in contract[:40]:
            print(f"    {path}")
    print("\nRecent commits:")
    for entry in report["commits"]:
        print(f"  {entry['sha']}  {entry['subject'][:96]}")
    print("\nThe procedure for moving the pin is in vendor/agent-desktop/README.md.")


if __name__ == "__main__":
    main()
