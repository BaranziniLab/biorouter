#!/usr/bin/env bash
# Assert the macOS text insertion helper (crates/biorouter-textinsert) is inside a packaged app,
# where the desktop app looks for it (`textInsertHelperPath` in
# ui/desktop/src/utils/textInsertHelper.ts: Contents/Resources/bin/biorouter-textinsert), built for
# the app's architecture, and signed with the app's own Team ID.
#
# A missing or wrong-architecture helper packages cleanly and breaks nothing visible except
# insertion: every dictation into another app is copied instead, with a "could not type" notice.
# A helper signed by anyone else would be a program that types into other apps under Biorouter's
# Accessibility grant. Only opening the built .app catches any of it, so that is what this does.
set -euo pipefail

APP="${1:?usage: check-text-insert-helper-bundled.sh <path to Biorouter.app>}"
DAEMON="$APP/Contents/Resources/bin/biorouterd"
HELPER="$APP/Contents/Resources/bin/biorouter-textinsert"

if [ ! -x "$HELPER" ]; then
  echo "FAIL: no executable text insertion helper at $HELPER (ui/desktop/scripts/build-text-insert-helper.mjs)"
  exit 1
fi
WANT=$(file -b "$DAEMON" | grep -oE 'arm64|x86_64' | head -1)
GOT=$(file -b "$HELPER" | grep -oE 'arm64|x86_64' | head -1)
if [ "$WANT" != "$GOT" ]; then
  echo "FAIL: daemon is $WANT but the text insertion helper is $GOT"
  exit 1
fi

# The helper must carry the same signing team as the daemon beside it. An unsigned app (a local
# package without APPLE_ID) has no team on either, which matches.
team() {
  { codesign -dv "$1" 2>&1 || true; } | sed -n 's/^TeamIdentifier=//p' | head -1
}
DAEMON_TEAM=$(team "$DAEMON")
HELPER_TEAM=$(team "$HELPER")
if [ "$DAEMON_TEAM" != "$HELPER_TEAM" ]; then
  echo "FAIL: daemon is signed by team '${DAEMON_TEAM:-none}' but the text insertion helper by '${HELPER_TEAM:-none}'"
  exit 1
fi
echo "OK: text insertion helper found where the app looks ($GOT, team ${HELPER_TEAM:-none})"
