#!/usr/bin/env bash
# BR-70: assert the cross-built linux binaries do not require a glibc newer than
# the floor (default 2.31 / Debian 11 bullseye). A `cargo check` cannot see this
# — checks do not link, so no symbol versions are emitted — so this runs after
# the nightly full cross BUILD (and locally before a release). It is the check
# that would have MECHANICALLY caught the `rust:latest` (trixie, glibc 2.39)
# regression the release comments describe, instead of a human noticing a
# container smoke test failed.
#
# Usage: scripts/check-glibc-floor.sh [--max-out FILE] [BIN_DIR [BINARY...]]
#   BIN_DIR defaults to target/x86_64-unknown-linux-gnu/release; a relative
#   BIN_DIR resolves from the repository root.
#   BINARY defaults to the three shipped binaries, biorouterd biorouter
#   biorouter-crew. Naming binaries checks only those, so a broker-only build
#   (scripts/stage-crew-broker.sh) gets this symbol-table read rather than a
#   weaker `strings` scan.
#   --max-out FILE writes the highest GLIBC version the binaries need (for
#   example `2.30`) to FILE, for a caller that records it. Pass an absolute
#   FILE: like BIN_DIR, a relative one resolves from the repository root.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=scripts/cross-env.sh
. "$(dirname "$0")/cross-env.sh"

MAX_OUT=""
if [ "${1:-}" = "--max-out" ]; then
  MAX_OUT="${2:?--max-out needs a file}"
  shift 2
fi
BIN_DIR="${1:-target/x86_64-unknown-linux-gnu/release}"
[ "$#" -gt 0 ] && shift
if [ "$#" -gt 0 ]; then
  BINARIES=("$@")
else
  BINARIES=(biorouterd biorouter biorouter-crew)
fi
for b in "${BINARIES[@]}"; do
  [ -f "$BIN_DIR/$b" ] || { echo "::error::missing binary $BIN_DIR/$b — run the cross build first"; exit 2; }
done
# The container sees BIN_DIR alone, read-only, so the directory may sit outside
# the checkout (a staging directory, a downloaded artifact).
BIN_DIR="$(cd "$BIN_DIR" && pwd)"

# readelf supports cross-architecture ELF and lives in the pinned image; the host needs no
# binutils. Extract every GLIBC_x.y symbol version the binaries import and keep
# the highest.
if ! versions=$(docker run --rm -v "$BIN_DIR":/w:ro -w /w "$LINUX_RUST_IMG" sh -ec '
    for binary do
      readelf --version-info "$binary" || exit 2
    done
  ' sh "${BINARIES[@]}"); then
  echo "::error::could not inspect every Linux binary's glibc versions" >&2
  exit 2
fi
worst=$(printf '%s\n' "$versions" | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -V | tail -1)

[ -n "$worst" ] || { echo "::error::could not read glibc symbol versions from $BIN_DIR"; exit 2; }

if [ "$(printf '%s\n%s\n' "$GLIBC_FLOOR" "$worst" | sort -V | tail -1)" != "$GLIBC_FLOOR" ]; then
  echo "::error::binaries require glibc $worst > floor $GLIBC_FLOOR — Debian 12 / Ubuntu 22.04 / Rocky 9 would fail to start"
  exit 1
fi
[ -z "$MAX_OUT" ] || printf '%s\n' "$worst" >"$MAX_OUT"
echo "OK — max glibc requirement $worst ≤ floor $GLIBC_FLOOR"
