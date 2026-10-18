#!/usr/bin/env bash
# Cross-build the Linux x86_64 Crew broker alone and stage it with its manifest (OW-9).
#
# The desktop app and the CLI packages carry the exact Linux broker that their Crew setup
# service installs on a server (see scripts/stage-crew-broker.sh). A release builds it with
# the rest of the Linux backend (`scripts/release.sh linux-backend`). This builds only
# `biorouter-crew`, which has no C dependencies and takes about a minute on a warm cache, for
# development (`just stage-crew-broker`) and for packaging jobs with no Linux backend to hand.
#
# Usage: scripts/build-crew-broker-linux.sh <out-dir>
#   <out-dir> is the staging directory to (re)create, for example
#   ui/desktop/src/bin/crew-broker/linux-x86_64. A relative path resolves from the caller's
#   directory.
#
# It compiles through `cross_linux` in scripts/cross-env.sh, the one cross recipe
# (scripts/check-no-cross-drift.sh fails a second copy), so the broker gets the release's
# pinned bullseye image and glibc floor. Default features stay on, so the broker keeps
# join-by-name (scripts/check-crew-broker-join.sh).
#
# The cargo target directory persists between runs: CROSS_TARGET_MOUNT when it is set (a host
# path or a Docker volume, as in cross-env.sh), else the Docker volume named by
# CREW_BROKER_TARGET_VOLUME (default biorouter-crew-broker-target). `release.sh` destroys its
# volume before every build, because cached objects can keep stale glibc symbol versions;
# keeping this cache is safe only because stage-crew-broker.sh reads the floor off the bytes
# it stages, on every run.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET=x86_64-unknown-linux-gnu

log() { printf '\033[1;36m[crew-broker]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[crew-broker] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

OUT="${1:?usage: build-crew-broker-linux.sh <out-dir>}"
mkdir -p "$(dirname "$OUT")"
OUT="$(cd "$(dirname "$OUT")" && pwd)/$(basename "$OUT")"

# shellcheck source=scripts/cross-env.sh
. "$ROOT/scripts/cross-env.sh"
docker info >/dev/null 2>&1 || die "the Docker daemon is not reachable — start Docker and retry"

: "${CROSS_TARGET_MOUNT:=${CREW_BROKER_TARGET_VOLUME:-biorouter-crew-broker-target}}"
export CROSS_TARGET_MOUNT
case "$CROSS_TARGET_MOUNT" in
  /*) mkdir -p "$CROSS_TARGET_MOUNT" ;;
  *) docker volume create "$CROSS_TARGET_MOUNT" >/dev/null ;;
esac

started="$(date +%s)"
log "cross-compiling biorouter-crew for $TARGET ($LINUX_RUST_IMG, target dir $CROSS_TARGET_MOUNT)"
cross_linux "cargo build --locked --release -p biorouter-crew --bin biorouter-crew" /cross-target \
  || die "the broker build failed (output above)"

# Copied out through the container's stdout rather than a second bind mount, so on a Linux
# host the file belongs to the caller, not to the container's root.
BUILT="$(mktemp -d)"
trap 'rm -rf -- "$BUILT"' EXIT
case "$CROSS_TARGET_MOUNT" in
  /*) cp "$CROSS_TARGET_MOUNT/$TARGET/release/biorouter-crew" "$BUILT/biorouter-crew" ;;
  *) docker run --rm -v "$CROSS_TARGET_MOUNT":/cross-target:ro "$LINUX_RUST_IMG" \
       cat "/cross-target/$TARGET/release/biorouter-crew" >"$BUILT/biorouter-crew" ;;
esac \
  || die "could not copy the built broker out of $CROSS_TARGET_MOUNT"
log "built in $(( $(date +%s) - started ))s"

bash "$ROOT/scripts/stage-crew-broker.sh" "$BUILT/biorouter-crew" "$OUT"
