#!/usr/bin/env bash
# Runs INSIDE a linux/amd64 Docker container to produce the Linux .deb package.
# Called by: just make-ui-linux (step 2/2)
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(dirname "$SCRIPT_DIR")"
UI_DIR="$(dirname "$DESKTOP_DIR")"
PROJECT_ROOT="$(dirname "$UI_DIR")"
BIN_DIR="$DESKTOP_DIR/src/bin"
LINUX_RELEASE="$PROJECT_ROOT/target/x86_64-unknown-linux-gnu/release"

echo "Installing system dependencies (fakeroot, dpkg, rpm)..."
bash "$SCRIPT_DIR/setup-linux-native-baseline.sh"
apt-get update -q
apt-get install -y --no-install-recommends fakeroot dpkg rpm

echo "Replacing macOS ARM binaries with Linux x64 binaries in src/bin/..."
# Remove macOS ARM executables — they are non-functional on Linux
for f in biorouterd biorouter jbang npx uvx node; do
    if [ -f "$BIN_DIR/$f" ]; then
        rm -f "$BIN_DIR/$f"
        echo "  Removed macOS binary: $f"
    fi
done
# Remove Windows executables and DLLs — they do not belong in Linux packages.
#
# ⚠ RECURSIVE, deliberately. This was a top-level glob (`$BIN_DIR/*.exe`), which
# cannot match `$BIN_DIR/llamacpp/llama-server.exe` — and that is exactly where
# they were. The shipped .deb carried 31 Windows files under
# resources/bin/llamacpp/ as a result. `prepare-platform-binaries.js` below
# replaces that whole directory with the Linux sidecar and then asserts no
# foreign executable survives anywhere, so this sweep is defence in depth rather
# than the only guard.
find "$BIN_DIR" \( -name '*.exe' -o -name '*.dll' -o -name '*.cmd' \) -type f -print -delete 2>/dev/null || true
# Remove bundled MinGit (Windows-only Git distribution dropped by download-mingit.js)
if [ -d "$BIN_DIR/git" ]; then
    rm -rf "$BIN_DIR/git"
    echo "  Removed MinGit directory: git/"
fi

# Copy Linux x64 Rust binaries
if [ ! -f "$LINUX_RELEASE/biorouter" ] || [ ! -f "$LINUX_RELEASE/biorouterd" ]; then
    echo "ERROR: Linux binaries not found at $LINUX_RELEASE"
    echo "       Run step 1 (Rust cross-compilation) before this step."
    exit 1
fi
cp "$LINUX_RELEASE/biorouter" "$BIN_DIR/biorouter"
chmod +x "$BIN_DIR/biorouter"
echo "  Installed Linux x64: biorouter"
cp "$LINUX_RELEASE/biorouterd" "$BIN_DIR/biorouterd"
chmod +x "$BIN_DIR/biorouterd"
echo "  Installed Linux x64: biorouterd"

cd "$DESKTOP_DIR"
npm ci --cache /root/.npm

# Run the SAME preparation every other platform runs. This script used to call
# `npm run make` directly and skip it entirely, which is why the Linux packages
# were the only ones with no binary validation — and why they shipped a Windows
# llama-server: nothing here fetched the Linux one, so whatever the macOS build
# had left in src/bin/llamacpp/ was packaged as-is.
#
# It fetches the Linux sidecar (replacing that directory wholesale), builds the
# browser interface bundle biorouterd serves (BIOROUTER_SERVE_UI, shipped as the
# extraResource 'src/web'), asserts the required binaries are present, and
# asserts no foreign executable survived.
#
# The deb and rpm install under /opt, so `<exe dir>/../web` resolves inside the
# app tree exactly as it does on macOS and Windows. It is the CLI-only packages
# (packaging/biorouter-cli.yaml, /usr/bin) that cannot use that rule and place the
# bundle at /usr/share/biorouter/web instead.
echo "Preparing Linux platform binaries (llama-server, web bundle, validation)..."
ELECTRON_PLATFORM=linux ELECTRON_ARCH=x64 node scripts/prepare-platform-binaries.js

echo "Running electron-forge make for Linux x64 (deb, rpm, zip)..."
ELECTRON_PLATFORM=linux ELECTRON_ARCH=x64 npm run make -- --platform=linux --arch=x64 \
    --targets "@electron-forge/maker-deb,@electron-forge/maker-rpm"

echo ""
echo "Done! .deb package is at:"
echo "  $DESKTOP_DIR/out/make/deb/x64/"
