#!/usr/bin/env bash
# Live notarized auto-update SWAP test.
#
# Proves the one step that headless tests cannot: macOS Squirrel.Mac validates a
# downloaded *notarized* build's code signature and performs the in-place swap +
# relaunch when the app calls quitAndInstall.
#
# Setup (built beforehand by the caller):
#   $NEW_ZIP  — signed+notarized Biorouter.app zip at the NEW version (payload)
#   $OLD_APP  — signed+notarized Biorouter.app at the OLD version (under test)
#
# Flow: serve NEW + latest-mac.yml over local HTTP → run a copy of OLD pointed at
# that feed with BIOROUTER_UPDATE_AUTO_INSTALL=1 → wait for OLD to quit, ShipIt
# to install, and ShipIt to RELAUNCH the app.
#
# ⚠ It used to stop at the swap: PASS meant the on-disk version flipped, which
# a ShipIt that installed and never relaunched also produces. "The app doesn't
# restart after updating" was reported against releases this test had passed.
# PASS now needs ShipIt's own "Successfully launched application" line for the
# staged copy, written after this run started.
#
# FEED_VER (default NEW_VER) is the version the served manifest claims. Set it
# above OLD_VER and point NEW_ZIP at the SAME build as OLD_APP to check the
# whole install and relaunch with one signed build: Squirrel.Mac checks the
# update's signature, not its version. The on-disk version then stays put, so
# only the relaunch decides.
#
# ⚠ The test app shares ShipIt's state with any installed Biorouter
# (~/Library/Caches/com.electron.biorouter.ShipIt): an update the installed app
# had staged is replaced, and that app downloads it again at its next check.
# The relaunched copy runs without this script's environment, so if Biorouter
# is already open it hands over to that instance and exits.
#
# Config is sandboxed via XDG_CONFIG_HOME so the user's real ~/.config/biorouter
# is never touched.
set -uo pipefail

DESK="$(cd "$(dirname "$0")/.." && pwd)"
NEW_VER="${NEW_VER:-1.86.0}"
OLD_VER="${OLD_VER:-1.85.5}"
NEW_ZIP="${NEW_ZIP:-$DESK/out/make/zip/darwin/arm64/Biorouter-darwin-arm64-$NEW_VER.zip}"
OLD_APP="${OLD_APP:-$DESK/test-builds/old/Biorouter.app}"
PORT="${PORT:-8799}"
FEED_VER="${FEED_VER:-$NEW_VER}"
SHIPIT_LOG="$HOME/Library/Caches/com.electron.biorouter.ShipIt/ShipIt_stderr.log"
SHIPIT_LINES_BEFORE="$( [ -f "$SHIPIT_LOG" ] && wc -l < "$SHIPIT_LOG" | tr -d ' ' || echo 0 )"

WORK="$(mktemp -d /tmp/br-swap.XXXXXX)"
SERVE="$WORK/serve"; RUNDIR="$WORK/run"; CFG="$WORK/config"
mkdir -p "$SERVE" "$RUNDIR" "$CFG/biorouter"

# Seed the sandboxed config with the real (small) config so the backend starts
# and the app reaches the updater — but NOT the 624MB extensions, and never the
# user's real ~/.config (left untouched).
REAL_CFG="$HOME/.config/biorouter"
[ -f "$REAL_CFG/config.yaml" ] && cp "$REAL_CFG/config.yaml" "$CFG/biorouter/" 2>/dev/null
for d in skills workflows databricks; do
  [ -d "$REAL_CFG/$d" ] && cp -R "$REAL_CFG/$d" "$CFG/biorouter/" 2>/dev/null
done

cleanup() {
  [ -n "${SRV_PID:-}" ] && kill "$SRV_PID" 2>/dev/null
  [ -n "${APP_PID:-}" ] && kill "$APP_PID" 2>/dev/null
  pkill -f "$RUNDIR/Biorouter.app" 2>/dev/null
}
trap cleanup EXIT

echo "==> building update feed (latest-mac.yml + zip) from $NEW_ZIP"
[ -f "$NEW_ZIP" ] || { echo "FAIL: NEW_ZIP missing: $NEW_ZIP"; exit 1; }
cp "$NEW_ZIP" "$SERVE/"
( cd "$DESK" && node scripts/generate-update-manifests.js \
    --version "$FEED_VER" --arm64-zip "$SERVE/$(basename "$NEW_ZIP")" --out "$SERVE" >/dev/null )
[ -f "$SERVE/latest-mac.yml" ] || { echo "FAIL: manifest not generated"; exit 1; }
echo "    manifest:"; sed 's/^/      /' "$SERVE/latest-mac.yml"

echo "==> staging OLD app ($OLD_VER) into writable run dir"
[ -d "$OLD_APP" ] || { echo "FAIL: OLD_APP missing: $OLD_APP"; exit 1; }
cp -R "$OLD_APP" "$RUNDIR/"
APP="$RUNDIR/Biorouter.app"
PLIST="$APP/Contents/Info.plist"
BEFORE="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$PLIST")"
echo "    staged version before update: $BEFORE"
[ "$BEFORE" = "$OLD_VER" ] || echo "    (note: expected $OLD_VER)"

echo "==> starting local update server on :$PORT"
( cd "$SERVE" && python3 -m http.server "$PORT" >/dev/null 2>&1 ) &
SRV_PID=$!
sleep 1
curl -fsS "http://127.0.0.1:$PORT/latest-mac.yml" >/dev/null || { echo "FAIL: server not serving"; exit 1; }

echo "==> launching OLD app pointed at local feed (auto-install on download)"
BIN="$APP/Contents/MacOS/Biorouter"
USERDATA="$WORK/userdata"; mkdir -p "$USERDATA"
APPLOG="$USERDATA/logs/main.log"   # electron-log writes here (packaged builds)
# --user-data-dir isolates this test instance's state (logs, updater cache,
# window state) from the user's running Biorouter — no shared-state collisions.
XDG_CONFIG_HOME="$CFG" \
BIOROUTER_DISABLE_KEYRING=true \
BIOROUTER_UPDATE_FEED_URL="http://127.0.0.1:$PORT" \
BIOROUTER_UPDATE_AUTO_INSTALL=1 \
ENABLE_DEV_UPDATES=true \
  "$BIN" --user-data-dir="$USERDATA" >"$WORK/stdout.log" 2>&1 &
APP_PID=$!

# New ShipIt lines since this run started.
shipit_since() { [ -f "$SHIPIT_LOG" ] && tail -n +"$((SHIPIT_LINES_BEFORE + 1))" "$SHIPIT_LOG" || true; }

echo "==> waiting for quit, install and relaunch (up to ~180s)"
RESULT="timeout"
EXITED="no"
for i in $(seq 1 36); do
  sleep 5
  NOW="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$PLIST" 2>/dev/null || echo '?')"
  kill -0 "$APP_PID" 2>/dev/null || EXITED="yes"
  echo "    [$((i*5))s] on-disk version = $NOW, old process exited = $EXITED"
  if shipit_since | grep -q "Successfully launched application at file://$APP/"; then
    RESULT="relaunched"; break
  fi
done

echo "==> updater log excerpt:"
grep -iE "feed override|auto-updater|update-(available|downloaded)|checking|squirrel|quitAndInstall|download-progress|AUTO_INSTALL|error" "$APPLOG" 2>/dev/null | tail -30 | sed 's/^/      /'
[ -s "$APPLOG" ] || echo "      (no electron-log produced — app may not have started; stdout below)"
[ -s "$WORK/stdout.log" ] && { echo "      --- stdout ---"; tail -15 "$WORK/stdout.log" | sed 's/^/      /'; }

echo "==> ShipIt log for this run:"
shipit_since | grep -v "^ " | tail -20 | sed 's/^/      /'

AFTER="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$PLIST" 2>/dev/null || echo '?')"
echo
echo "==================== RESULT ===================="
echo "  before: $BEFORE   after: $AFTER   old process exited: $EXITED   outcome: $RESULT"
if [ "$RESULT" = "relaunched" ] && [ "$AFTER" = "$NEW_VER" ]; then
  echo "  PASS: installed $AFTER in place and relaunched it"
  exit 0
fi
[ "$AFTER" != "$NEW_VER" ] && echo "  FAIL: the app on disk is $AFTER, not $NEW_VER"
[ "$RESULT" != "relaunched" ] && echo "  FAIL: ShipIt did not relaunch the app"
echo "  (app log at $APPLOG — kept for inspection)"
trap - EXIT; cleanup
exit 1
