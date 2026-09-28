#!/usr/bin/env bash
# Copy the agent-driven-UI example apps into the local Biorouter store, wiring in
# the current App SDK so they run against this checkout.
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SRC="$REPO/scripts/agent-drafter-apps/examples/ui"
STORE="${XDG_CONFIG_HOME:-$HOME/.config}/biorouter/agent_drafter"
SDK="$REPO/crates/biorouter-mcp/src/agent_drafter/templates/sdk.ts"

for app in "$SRC"/*/; do
  id="$(basename "$app")"
  [ -f "$app/manifest.json" ] || continue
  mkdir -p "$STORE/$id/src"
  cp "$app/manifest.json" "$app/index.html" "$STORE/$id/"
  cp "$app/src/main.ts" "$STORE/$id/src/main.ts"
  cp "$SDK" "$STORE/$id/src/sdk.ts"
  rm -rf "$STORE/$id/dist"   # force a rebuild against the current SDK
  echo "installed $id"
done
echo "Done. Open one with: biorouter apps open <id>"
