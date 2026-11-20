#!/usr/bin/env bash
# Build one non-macOS Biorouter Copilot helper payload (agent-desktop) into
# target/computer-use/<target>, inside the pinned rust:1.92-bullseye image, then
# verify it on the host. The recipe lives in scripts/cross-env.sh
# (cross_copilot_helper); macOS payloads are built on macOS with
# `python3 scripts/computer-use-runtime.py build darwin-<arch>`.
#
# Usage: scripts/build-computer-use-helper-cross.sh linux-x64|linux-arm64|win32-x64
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=scripts/cross-env.sh
. scripts/cross-env.sh
target="${1:?usage: scripts/build-computer-use-helper-cross.sh linux-x64|linux-arm64|win32-x64}"
cross_copilot_helper "$target"
python3 scripts/computer-use-runtime.py verify "$target"
