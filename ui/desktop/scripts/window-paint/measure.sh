#!/bin/bash
# measure.sh <electron-pid> <out-dir> <canvas R,G,B> <driver> [args...]
#
# Does the Biorouter window ever show a colour the app does not paint while it is
# resized or moved? Captures ONLY that window at ~50 frames/s (wincap) while one driver
# runs, and prints how many frames carried an unpainted band. It cannot see a late
# frame whose uncovered part is the canvas colour, so 0 does not mean "no lag". See
# docs/desktop-ui/window-scaling-regressions.md, "Unpainted window area".
#
# drivers
#   size  W0 H0 W1 H1 STEPS MS   stepped Accessibility resize (live-drag-like)
#   pos   X0 Y0 X1 Y1 STEPS MS   stepped move
#   stall SECS W H               stop the app's GPU process for SECS around ONE resize
#                                to W x H: a compositor that is late, as on a loaded
#                                machine, made long enough to measure. Resumed on exit.
#
# canvas: what the capture reports for --background-app — 255,255,255 light,
# 20,20,19 dark on a standard sRGB display. A band is only detectable when it differs
# from the canvas, so measure in DARK mode: in light mode the old white band was
# invisible to this rule and to people alike, except where it replaced the sidebar.
#
# Use your OWN instance (launch-dev-gui.sh), never another agent's: the stall driver
# freezes the GPU process of the pid you name. Keep the window at least partly
# uncovered — a fully covered window is hidden, stops producing frames, and measures
# something else (the renderer's stale frame, extended with the page's own colour).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
PID=$1; OUT=$2; CANVAS=$3; DRIVER=$4; shift 4
BIN="${TMPDIR:-/tmp}/biorouter-window-paint"
mkdir -p "$BIN" "$OUT"
for tool in wincap axdrive; do
  if [ ! -x "$BIN/$tool" ] || [ "$here/$tool.swift" -nt "$BIN/$tool" ]; then
    swiftc -O "$here/$tool.swift" -o "$BIN/$tool"
  fi
done
echo "uptime: $(uptime)"
"$BIN/wincap" "$PID" "$OUT" "${CAPTURE_MS:-3500}" "$CANVAS" > "$OUT/wincap.txt" &
CAP=$!
sleep 0.3
case "$DRIVER" in
  size | pos) "$BIN/axdrive" "$PID" "$DRIVER" "$@" > "$OUT/driver.txt" ;;
  stall)
    SECS=$1; W=$2; H=$3
    GPU=""
    for p in $(pgrep -P "$PID"); do
      ps -o command= -p "$p" | grep -q -- '--type=gpu-process' && GPU=$p
    done
    [ -n "$GPU" ] || { echo "no GPU process under pid $PID"; kill "$CAP"; exit 1; }
    trap 'kill -CONT "$GPU" 2>/dev/null || true' EXIT
    kill -STOP "$GPU"
    "$BIN/axdrive" "$PID" size "$W" "$H" "$W" "$H" 0 0 > "$OUT/driver.txt"
    sleep "$SECS"
    kill -CONT "$GPU"
    ;;
  *) echo "unknown driver $DRIVER"; kill "$CAP"; exit 2 ;;
esac
wait "$CAP"
echo "driver: $(head -1 "$OUT/driver.txt") ... $(tail -1 "$OUT/driver.txt")"
cat "$OUT/wincap.txt"
