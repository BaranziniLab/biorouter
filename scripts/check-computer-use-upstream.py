#!/usr/bin/env python3
"""Report what has changed upstream since the vendored Computer Use commit.

Read-only. It never writes to the tree, never changes the pin, and never applies
anything — deciding to take an upstream change is a review, not a command.

    python3 scripts/check-computer-use-upstream.py            # summary
    python3 scripts/check-computer-use-upstream.py --json     # machine-readable
    python3 scripts/check-computer-use-upstream.py --limit 50

⚠ The useful output is not "you are N commits behind". It is **which upstream
files our patches also touch**, because those are the ones that will conflict
when the tree is re-vendored. A patch that no longer applies is the whole cost of
an upstream update, and this is what predicts it before anyone starts.

Updating, once you have read this:

    1. git clone <repository> /tmp/ocu && git -C /tmp/ocu checkout <new commit>
    2. edit third_party/open-computer-use/pin.json  (upstream_commit, upstream_version)
    3. python3 scripts/vendor-computer-use-source.py --from /tmp/ocu
    4. python3 scripts/computer-use-runtime.py build <target>   # patches re-apply here
    5. resolve any patch that no longer applies, bump patch_revision in pin.json
    6. rebuild every target and re-run the acceptance scripts
"""
import argparse
import json
import re
import subprocess
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "third_party/open-computer-use"
API = "https://api.github.com"


def owner_repo(url):
    match = re.search(r"github\.com[/:]([^/]+)/([^/.]+)", url)
    if not match:
        raise ValueError(f"cannot read an owner/repo out of {url!r}")
    return match.group(1), match.group(2)


def fetch(url):
    request = urllib.request.Request(url, headers={
        "Accept": "application/vnd.github+json",
        "User-Agent": "biorouter-computer-use-upstream-check",
    })
    # A token lifts the 60/hour anonymous rate limit; it is optional and never required.
    token = subprocess.run(["gh", "auth", "token"], capture_output=True, text=True)
    if token.returncode == 0 and token.stdout.strip():
        request.add_header("Authorization", f"Bearer {token.stdout.strip()}")
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def patched_paths():
    """Every upstream path our patches touch, read out of the patches themselves."""
    paths = set()
    for patch in sorted((VENDOR / "patches").glob("*.patch")):
        for line in patch.read_text(errors="replace").splitlines():
            if line.startswith(("--- a/", "+++ b/")):
                path = line[6:].strip()
                if path and path != "/dev/null":
                    paths.add(path)
    return paths


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--json", action="store_true", help="emit the report as JSON")
    parser.add_argument("--limit", type=int, default=30, help="how many commits to list")
    args = parser.parse_args()

    pin = json.loads((VENDOR / "pin.json").read_text())
    owner, repo = owner_repo(pin["repository"])
    pinned = pin["upstream_commit"]

    try:
        meta = fetch(f"{API}/repos/{owner}/{repo}")
        branch = meta.get("default_branch", "main")
        compare = fetch(f"{API}/repos/{owner}/{repo}/compare/{pinned}...{branch}")
    except urllib.error.HTTPError as error:
        raise SystemExit(
            f"upstream check failed: {error.code} {error.reason}. "
            "This is only a convenience — the vendored source in this repository "
            "is what builds, and it does not need the network."
        ) from None
    except urllib.error.URLError as error:
        raise SystemExit(f"upstream check could not reach GitHub: {error.reason}") from None

    commits = compare.get("commits", [])
    changed = {f["filename"] for f in compare.get("files", [])}
    ours = patched_paths()
    conflicts = sorted(changed & ours)

    report = {
        "repository": pin["repository"],
        "vendored_commit": pinned,
        "vendored_version": pin["upstream_version"],
        "patch_revision": pin["patch_revision"],
        "default_branch": branch,
        "behind_by": compare.get("behind_by", 0),
        "ahead_by": compare.get("ahead_by", len(commits)),
        "upstream_head": compare.get("commits", [{}])[-1].get("sha") if commits else pinned,
        "files_changed_upstream": len(changed),
        "files_our_patches_touch": sorted(ours),
        "likely_patch_conflicts": conflicts,
        "commits": [
            {"sha": c["sha"][:12], "subject": (c["commit"]["message"].splitlines() or [""])[0]}
            for c in commits[-args.limit:]
        ],
    }

    if args.json:
        print(json.dumps(report, indent=2))
        return

    print(f"vendored : {pinned[:12]}  (v{pin['upstream_version']}, patches {pin['patch_revision']})")
    print(f"upstream : {report['upstream_head'][:12]}  on {branch}")
    if not commits:
        print("\nUp to date — nothing upstream since the vendored commit.")
        return
    print(f"\n{len(commits)} commit(s) upstream, touching {len(changed)} file(s):\n")
    for entry in report["commits"]:
        print(f"  {entry['sha']}  {entry['subject'][:96]}")
    if len(commits) > len(report["commits"]):
        print(f"  … {len(commits) - len(report['commits'])} older commit(s) not shown (--limit)")
    print()
    if conflicts:
        print(f"⚠ {len(conflicts)} of those file(s) are ones OUR PATCHES ALSO TOUCH.")
        print("  Re-vendoring will very likely need these patches reworked:")
        for path in conflicts:
            print(f"    {path}")
    else:
        print("No upstream change touches a file our patches touch, so the patches")
        print("are likely to re-apply cleanly. Verify by building, not by assuming.")
    print("\nSee this script's header for the update procedure.")


if __name__ == "__main__":
    main()
