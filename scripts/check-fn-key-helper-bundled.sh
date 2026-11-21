#!/usr/bin/env bash
# Assert the macOS fn key helper (crates/biorouter-fnkey) is inside a packaged app, where the
# desktop app looks for it (`fnKeyHelperPath` in ui/desktop/src/utils/fnKeyHelper.ts:
# Contents/Resources/bin/biorouter-fnkey), and built for the app's architecture.
#
# A missing or wrong-architecture helper packages cleanly and breaks nothing visible except the
# shortcut: Settings shows MSG-FN-MISSING, or the helper never starts on an Intel Mac. Only
# opening the built .app catches it, so that is what this does.
set -euo pipefail

APP="${1:?usage: check-fn-key-helper-bundled.sh <path to Biorouter.app>}"
DAEMON="$APP/Contents/Resources/bin/biorouterd"
HELPER="$APP/Contents/Resources/bin/biorouter-fnkey"

if [ ! -x "$HELPER" ]; then
  echo "FAIL: no executable fn key helper at $HELPER (ui/desktop/scripts/build-fn-key-helper.mjs)"
  exit 1
fi
WANT=$(file -b "$DAEMON" | grep -oE 'arm64|x86_64' | head -1)
GOT=$(file -b "$HELPER" | grep -oE 'arm64|x86_64' | head -1)
if [ "$WANT" != "$GOT" ]; then
  echo "FAIL: daemon is $WANT but the fn key helper is $GOT"
  exit 1
fi
echo "OK: fn key helper found where the app looks ($GOT)"
