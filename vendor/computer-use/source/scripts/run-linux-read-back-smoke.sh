#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$(uname -s)" != Linux || $# -lt 1 ]]; then
  echo "Usage (Linux): bash scripts/run-linux-read-back-smoke.sh /path/to/open-computer-use [--focus-checks | --hidden-checks | --observation-checks]" >&2
  exit 2
fi

native_path="$1"
shift
runtime_dir="$(mktemp -d)"
trap 'rm -rf "${runtime_dir}"' EXIT
XDG_RUNTIME_DIR="${runtime_dir}" NO_AT_BRIDGE=0 GTK_A11Y=always \
  xvfb-run -a dbus-run-session -- python3 \
  "${repo_root}/scripts/tests/linux-read-back-smoke.py" --native "${native_path}" "$@"
