#!/usr/bin/env bash
# Drive Biorouter Copilot's GTK fixture app through Copilot's own tools (AD-21), in a throwaway
# X11 session: Xvfb, a session bus with the accessibility bridge (at-spi2-core) and openbox,
# which the headed actions need to focus a window. Never run it on a person's desktop.
#
#   scripts/test-copilot-fixture-linux.sh PAYLOAD_DIR
#
# PAYLOAD_DIR is a built engine payload (target/computer-use/linux-x64 or linux-arm64). The
# driver is crates/biorouter-mcp/tests/computer_use_fixture.rs, run with cargo, or the test
# binary BIOROUTER_COPILOT_FIXTURE_DRIVER names when it is already built (how the Docker
# reproduction in docs/testing/copilot-fixture-acceptance.md runs it). Needs xvfb-run,
# dbus-run-session, openbox, at-spi2-core, python3-gi and gir1.2-gtk-3.0.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "${1:-}" != "--inside" ]; then
  payload="${1:?usage: scripts/test-copilot-fixture-linux.sh PAYLOAD_DIR}"
  payload="$(cd "$payload" && pwd)"
  exec xvfb-run -a -s "-screen 0 1280x800x24" dbus-run-session -- "$0" --inside "$payload"
fi
payload="$2"

export NO_AT_BRIDGE=0 GTK_MODULES=atk-bridge XDG_SESSION_TYPE=x11
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-$(mktemp -d)}"
chmod 700 "$XDG_RUNTIME_DIR"
export BIOROUTER_COMPUTER_USE_DIR="$payload"
export BIOROUTER_COPILOT_FIXTURE='["python3","scripts/copilot-fixture-gtk.py"]'

openbox --sm-disable >/dev/null 2>&1 &
manager=$!
trap 'kill "$manager" 2>/dev/null || true' EXIT
sleep 1

if [ -n "${BIOROUTER_COPILOT_FIXTURE_DRIVER:-}" ]; then
  "$BIOROUTER_COPILOT_FIXTURE_DRIVER" --ignored --nocapture --test-threads=1
else
  cargo test --locked -p biorouter-mcp --test computer_use_fixture -- --ignored --nocapture --test-threads=1
fi
