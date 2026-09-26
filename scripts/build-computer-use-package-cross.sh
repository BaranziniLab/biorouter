#!/usr/bin/env bash
# Build the actual shipped GNU binaries with the release's centralized recipes.
set -euo pipefail
cd "$(dirname "$0")/.."
. scripts/cross-env.sh
target="${1:?target triple required}"
mkdir -p ".cross-cache/target/$target" .cross-cache/registry "target/$target/release"
export CROSS_TARGET_MOUNT="$PWD/.cross-cache/target/$target"
export CROSS_REGISTRY_MOUNT="$PWD/.cross-cache/registry"
post="cp /cross-target/$target/release/biorouter* /usr/src/myapp/target/$target/release/"
case "$target" in
  x86_64-unknown-linux-gnu)
    cross_linux 'cargo build --release --locked --bin biorouter --bin biorouterd --bin biorouter-crew -j 2' /cross-target "$post"
    bash scripts/check-glibc-floor.sh
    bash scripts/check-linux-runtime-deps.sh
    bash scripts/check-crew-broker-join.sh
    ;;
  x86_64-pc-windows-gnu)
    cross_windows 'cargo build --release --locked --bin biorouter --bin biorouterd -j 2' /cross-target "$post && $WIN_DLL_STAGE"
    ;;
  *) echo "Unsupported package target: $target" >&2; exit 2 ;;
esac
git rev-parse HEAD > "target/$target/release/package-source-commit.txt"
