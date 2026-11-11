#!/usr/bin/env bash
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=biorouter-launch.sh
. "$DIR/biorouter-launch.sh"

install_app
install_payload          # no-op for launcher-mode exports (no payload/ dir)
start_daemon
launch_url
open_url "$LAUNCH_URL"
