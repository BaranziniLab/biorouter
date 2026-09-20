#!/usr/bin/env bash
# The vendored Open Computer Use source must match its manifest, and must survive
# a checkout on any platform unchanged.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 scripts/vendor-computer-use-source.py --verify
python3 scripts/check-vendored-computer-use.py
