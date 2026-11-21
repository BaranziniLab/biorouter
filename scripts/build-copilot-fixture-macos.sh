#!/usr/bin/env bash
# Build Biorouter Copilot's macOS fixture app (scripts/copilot-fixture-macos.swift) as a real,
# ad hoc signed .app bundle. It must be a bundle: an accessibility client resolves an app by
# its bundle, and a loose binary has none.
#
# Usage: scripts/build-copilot-fixture-macos.sh [output-dir]   (default target/copilot-fixture)
set -euo pipefail
cd "$(dirname "$0")/.."
output=${1:-target/copilot-fixture}
bundle="$output/Biorouter Copilot Fixture.app"
rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS"
xcrun swiftc -O -target "$(uname -m)-apple-macos13.0" \
  -o "$bundle/Contents/MacOS/BiorouterCopilotFixture" scripts/copilot-fixture-macos.swift
cat > "$bundle/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>org.biorouter.copilot-fixture</string>
  <key>CFBundleName</key><string>Biorouter Copilot Fixture</string>
  <key>CFBundleDisplayName</key><string>Biorouter Copilot Fixture</string>
  <key>CFBundleExecutable</key><string>BiorouterCopilotFixture</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSPrincipalClass</key><string>NSApplication</string>
</dict>
</plist>
PLIST
codesign --force --sign - "$bundle"
echo "$bundle"
