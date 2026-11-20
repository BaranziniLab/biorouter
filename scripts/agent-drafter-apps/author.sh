#!/usr/bin/env bash
# Drive MiMo (via the biorouter CLI) to author one app through the Application Drafter
# tools. Usage: author.sh "<instruction>"
set -euo pipefail
cd /Users/wanjun/Desktop/biorouter-apps-wt
source bin/activate-hermit 2>/dev/null
export BIOROUTER_ESBUILD_BIN=/Users/wanjun/Desktop/biorouter/ui/desktop/node_modules/.bin/esbuild
export XIAOMI_MIMO_API_KEY=$(cat /tmp/br-mimo.key 2>/dev/null)
export XIAOMI_MIMO_HOST="https://token-plan-sgp.xiaomimimo.com/v1"
exec /tmp/br-apps-target/debug/biorouter run --with-builtin agent_drafter -t "$1"
