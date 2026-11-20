#!/usr/bin/env bash
# The vendored agent-desktop source (Biorouter Copilot's engine, AD-02) must match
# its manifest, name no network client crate (AD-20), and survive a checkout on any
# platform unchanged. The file keeps its old name because the Justfile, rust.yml and
# AGENTS.md run it by that path.
set -euo pipefail
cd "$(dirname "$0")/.."

python3 scripts/vendor-computer-use-source.py --verify
python3 scripts/check-vendored-computer-use.py
