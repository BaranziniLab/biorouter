#!/usr/bin/env bash
# Build the CLI-only Linux packages (.deb + .rpm) — just the headless
# `biorouter` CLI, `biorouterd` daemon and Crew SSH broker, no Electron/GUI.
#
# Prereqs: the linux-gnu binaries must already exist (built by
# `scripts/release.sh backends <ver>`):
#   target/x86_64-unknown-linux-gnu/release/{biorouter,biorouterd,biorouter-crew}
#
# The browser interface bundle is built HERE, on the host, because nothing else
# in the CLI-only path runs npm. That makes this script share the GUI packaging
# phases' ordering constraint: a host-native ui/desktop/node_modules is
# required, so run it BEFORE `release.sh linux`/`windows` (which leave a
# Linux-flavoured tree behind) or re-run `npm ci` in between.
#
# Usage: scripts/build-cli-linux-packages.sh <version>
# Output:
#   dist/cli/biorouter-cli_<version>_amd64.deb
#   dist/cli/biorouter-cli-<version>-1.x86_64.rpm
#
# Each package is then smoke-tested in a clean container (install + run
# `biorouter --version` and `biorouter doctor`) so we only ship functional
# artifacts.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:?usage: build-cli-linux-packages.sh <version>}"
REL="target/x86_64-unknown-linux-gnu/release"
DESK="ui/desktop"
WEB="$DESK/src/web"
OUT="dist/cli"
DEB="$OUT/biorouter-cli_${VERSION}_amd64.deb"
RPM="$OUT/biorouter-cli-${VERSION}-1.x86_64.rpm"
NFPM_IMAGE="goreleaser/nfpm:latest"

log() { printf '\033[1;36m[cli-pkg]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[cli-pkg] %s\033[0m\n' "$*" >&2; exit 1; }
BR_HINT_LABEL="cli-pkg"
# shellcheck source=scripts/lib/dependency-hint.sh
. "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/dependency-hint.sh"


br_require_command docker "The deb and rpm are built inside a container."
docker info >/dev/null 2>&1 || br_dependency_die docker "docker daemon is not running" \
  "The docker CLI is installed but cannot reach a daemon. Start Docker Desktop (or dockerd) and retry."
br_require_command npm "The browser interface bundle is built with npm run build:web."
[ -f "$REL/biorouter" ]  || die "missing $REL/biorouter — run: scripts/release.sh backends $VERSION"
[ -f "$REL/biorouterd" ] || die "missing $REL/biorouterd — run: scripts/release.sh backends $VERSION"
[ -f "$REL/biorouter-crew" ] || die "missing $REL/biorouter-crew — run: scripts/release.sh backends $VERSION"
# The bytes about to be packaged, not the ones a build step once checked: a broker without the
# default `join-by-name` feature passes every smoke test below and lets nobody join by invitation.
bash "$ROOT/scripts/check-crew-broker-join.sh" "$REL/biorouter-crew" \
  || die "$REL/biorouter-crew was built without the join-by-name feature"

python3 "$ROOT/scripts/computer-use-runtime.py" verify linux-x64

mkdir -p "$OUT"
rm -f "$DEB" "$RPM"

# ── 0. Build the browser interface bundle (shipped at /usr/share/biorouter/web)
# `biorouterd` serves this to a browser when `biorouter serve` points it there,
# so a CLI package without it can run a terminal session and nothing else.
# Always rebuilt: a stale src/web from an older checkout is as wrong as none.
log "building browser interface bundle ($WEB)"
( cd "$DESK" && npm run build:web )
[ -s "$WEB/index.html" ] || die "npm run build:web produced no $WEB/index.html"
[ -d "$WEB/assets" ] || die "npm run build:web produced no $WEB/assets — the bundle would serve a blank page"

# ── 1. Build deb + rpm with nfpm (no root needed) ─────────────────────────────
log "building deb + rpm with nfpm ($VERSION)"
docker run --rm -e VERSION="$VERSION" -v "$ROOT":/work -w /work "$NFPM_IMAGE" \
  package -f packaging/biorouter-cli.yaml -p deb -t "$DEB"
docker run --rm -e VERSION="$VERSION" -v "$ROOT":/work -w /work "$NFPM_IMAGE" \
  package -f packaging/biorouter-cli.yaml -p rpm -t "$RPM"

[ -f "$DEB" ] || die "deb was not produced"
[ -f "$RPM" ] || die "rpm was not produced"
log "deb: $DEB ($(du -h "$DEB" | cut -f1))"
log "rpm: $RPM ($(du -h "$RPM" | cut -f1))"

# ── 2. Smoke-test the .deb on a clean Debian system ───────────────────────────
log "smoke-testing .deb on debian:bookworm-slim"
docker run --rm --platform linux/amd64 -v "$ROOT/$OUT":/pkg debian:bookworm-slim bash -euxc '
  apt-get update -q
  apt-get install -y --no-install-recommends "/pkg/'"$(basename "$DEB")"'"
  test -x /usr/libexec/biorouter/computer-use/ocu
  test -s /usr/libexec/biorouter/computer-use/manifest.json
  /usr/libexec/biorouter/computer-use/ocu --version
  python3 -c "import gi; gi.require_version(\"Atspi\", \"2.0\"); gi.require_version(\"Gdk\", \"3.0\"); from gi.repository import Atspi, Gdk"
  command -v biorouter && command -v biorouterd && command -v biorouter-crew
  biorouter --version
  biorouterd --version
  crew_version=$(biorouter-crew --version)
  case "$crew_version" in "biorouter-crew "*) ;; *) exit 1 ;; esac
  biorouter-crew --help
  biorouter doctor --no-update --format json >/dev/null
  test -s /usr/share/biorouter/web/index.html
  test -n "$(ls -A /usr/share/biorouter/web/assets)"
  echo "DEB_SMOKE_OK"
' | grep -q DEB_SMOKE_OK || die "deb smoke test FAILED"
log "deb smoke test passed ✓"

# ── 3. Smoke-test the .rpm on a clean Rocky Linux system ──────────────────────
log "smoke-testing .rpm on rockylinux:9"
docker run --rm --platform linux/amd64 -v "$ROOT/$OUT":/pkg rockylinux:9 bash -euxc '
  dnf install -y "/pkg/'"$(basename "$RPM")"'"
  test -x /usr/libexec/biorouter/computer-use/ocu
  test -s /usr/libexec/biorouter/computer-use/manifest.json
  /usr/libexec/biorouter/computer-use/ocu --version
  python3 -c "import gi; gi.require_version(\"Atspi\", \"2.0\"); gi.require_version(\"Gdk\", \"3.0\"); from gi.repository import Atspi, Gdk"
  command -v biorouter && command -v biorouterd && command -v biorouter-crew
  biorouter --version
  biorouterd --version
  crew_version=$(biorouter-crew --version)
  case "$crew_version" in "biorouter-crew "*) ;; *) exit 1 ;; esac
  biorouter-crew --help
  biorouter doctor --no-update --format json >/dev/null
  test -s /usr/share/biorouter/web/index.html
  test -n "$(ls -A /usr/share/biorouter/web/assets)"
  echo "RPM_SMOKE_OK"
' | grep -q RPM_SMOKE_OK || die "rpm smoke test FAILED"
log "rpm smoke test passed ✓"

log "CLI-only Linux packages built and verified:"
log "  $DEB"
log "  $RPM"
