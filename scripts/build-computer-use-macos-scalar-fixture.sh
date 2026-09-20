#!/usr/bin/env bash
#
# Build the synthetic macOS scalar fixture as a real .app bundle.
#
# It must be a bundle, not a bare executable: an accessibility client resolves a
# target by bundle identifier, and a loose binary has none. The identifier below
# is what a Biorouter Copilot acceptance run names as its fixture.
#
#   scripts/build-computer-use-macos-scalar-fixture.sh [output-dir]
#   open "<output-dir>/BioRouter Scalar Fixture.app"
#
# The fixture writes one JSON line per observation to <output-dir>/events.jsonl.
# That log is the INDEPENDENT evidence: a `set_value` result is only believed
# when the fixture's own model, its cell and its knob rect agree with it. AX
# readback alone is never proof -- reading back a value an app never applied is
# exactly the confusion this fixture exists to make visible.

set -euo pipefail
cd "$(dirname "$0")/.."

output=${1:-target/computer-use/macos-scalar-fixture}
identifier=org.biorouter.synthetic-scalar-fixture
app="$output/BioRouter Scalar Fixture.app"

command -v swiftc >/dev/null || {
  echo "swiftc not found: install the Xcode command line tools" >&2
  exit 1
}

rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
swiftc -O -o "$app/Contents/MacOS/BioRouterScalarFixture" \
  scripts/computer-use-macos-scalar-fixture.swift

cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>BioRouterScalarFixture</string>
  <key>CFBundleIdentifier</key><string>$identifier</string>
  <key>CFBundleName</key><string>BioRouter Scalar Fixture</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleVersion</key><string>1</string>
  <key>LSUIElement</key><false/>
</dict>
</plist>
PLIST

codesign --force --sign - "$app" >/dev/null 2>&1 || {
  echo "warning: ad-hoc signing failed; macOS may refuse to grant it accessibility" >&2
}

echo "$app"
echo "bundle identifier: $identifier"
echo "event log: $output/events.jsonl (set BIOROUTER_SCALAR_FIXTURE_LOG to move it)"
