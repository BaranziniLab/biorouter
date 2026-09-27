#!/usr/bin/env bash
# Assert that a built Crew broker can let people join by invitation and device code.
#
# Joining by name (naming design slice S3a) is compiled only with the `biorouter-crew` cargo
# feature `join-by-name`, which is ON BY DEFAULT since 2026-09-25 (D17). Every build that ships
# the broker passes no feature flags, so it gets the feature. The way to lose it silently is
# `--no-default-features` on a build where nothing else turns it back on: a broker-only build
# (`cargo build -p biorouter-crew --no-default-features`), or any build after `biorouter` /
# `biorouter-server` stop taking this crate with its default features. (Today a combined `--bin biorouterd --bin biorouter --bin biorouter-crew`
# build keeps it even with `--no-default-features`, through feature unification; measured with
# `cargo tree -e features` on 2026-09-25.) Such a broker still starts, still passes `--version`
# and `--help` and every glibc and runtime-dependency check, and fails only when a host tries to
# invite someone: `hello` no longer announces `join_by_name_v1`, so every client falls back to
# the legacy enrollment token.
#
# The capability string is the marker: `broker.rs` pushes the literal `join_by_name_v1` only
# under `#[cfg(feature = "join-by-name")]`, and no other compiled code in the crate spells it,
# so it is present in the binary exactly when the feature is. Measured on 2026-09-25 (macOS
# debug builds): the default build carries it and this check passes; a `--no-default-features`
# broker does not, and this check exits 1.
#
# Usage: scripts/check-crew-broker-join.sh [BROKER_BINARY]
#   BROKER_BINARY defaults to target/x86_64-unknown-linux-gnu/release/biorouter-crew under the
#   repository root; a relative argument is resolved from the caller's directory.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

BIN="${1:-$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter-crew}"
[ -f "$BIN" ] || { echo "::error::missing Crew broker $BIN — build it first" >&2; exit 2; }
[ -s "$BIN" ] || { echo "::error::Crew broker $BIN is empty" >&2; exit 2; }

# `grep -c` prints 0 and exits 1 on no match, and exits 2 on a read error: keep the two apart so
# an unreadable file is never reported as a broker without the feature, or the reverse.
rc=0
count="$(LC_ALL=C grep -a -c 'join_by_name_v1' "$BIN")" || rc=$?
if [ "$rc" -gt 1 ]; then
  echo "::error::could not read $BIN" >&2
  exit 2
fi
if [ "${count:-0}" -lt 1 ]; then
  echo "::error::$BIN was built without the join-by-name feature: it does not announce join_by_name_v1, so nobody can join its workspaces by invitation. Rebuild without --no-default-features." >&2
  exit 1
fi
echo "OK — $BIN announces join_by_name_v1"
