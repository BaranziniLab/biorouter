#!/usr/bin/env bash
# The vendored Computer Use source must match its manifest, and every file in
# that manifest must actually be committable.
#
# ⚠ The second half is not redundant. Repository-wide `.gitignore` rules (`*.png`,
# `.agents/`, `.mcp.json`) matched 15 of the vendored files, and a tree that
# verifies locally while 15 files never reach the repository is the worst of both
# worlds: it builds here and fails on a fresh clone, with the manifest error
# pointing at the files rather than at the reason.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 scripts/vendor-computer-use-source.py --verify

ignored=$(python3 - <<'PY'
import json, subprocess
manifest = json.load(open('third_party/open-computer-use/source-manifest.json'))
paths = [f"third_party/open-computer-use/source/{e['path']}" for e in manifest['files']]
proc = subprocess.run(['git', 'check-ignore', '--stdin'], input="\n".join(paths),
                      capture_output=True, text=True)
print("\n".join(l for l in proc.stdout.splitlines() if l.strip()))
PY
)
if [ -n "$ignored" ]; then
  echo "These vendored files are gitignored and would never reach a fresh clone:" >&2
  printf '%s\n' "$ignored" | sed 's/^/  /' >&2
  echo "Add a negation to .gitignore; see the vendored-source block at its end." >&2
  exit 1
fi
echo "Vendored Computer Use source is intact and fully committable"
