#!/usr/bin/env bash
# Assert the shipped Linux binaries' RUNTIME LIBRARY CONTRACT: every shared
# library `biorouter` and `biorouterd` hard-link against is either part of the
# glibc base that every target distro already has, or is named as a dependency
# by the packages we actually ship.
#
# Why this needs a guard at all. The two binaries are not static. Their ELF
# DT_NEEDED entries determine the requirements of each build. Known non-glibc
# libraries include libxcb.so.1 from `arboard` (clipboard, biorouter-cli) and
# libz.so.1 from vendored libgit2 through `libz-sys`. NEEDED is not a soft
# dependency: the loader resolves each entry before `main` runs, so a box missing
# a required library gets `error while loading shared libraries` and exit 127 on
# `--version`. There is no lazy path and no degraded mode to fall back to.
#
# That is fine, and invisible to users, precisely BECAUSE the packages declare
# it — `packaging/biorouter-cli.yaml` names libxcb1 and zlib1g (deb) / libxcb and zlib
# (rpm), so apt and dnf install them alongside the binaries. The failure this
# script exists to catch is the day those two facts drift apart: someone adds a
# crate that links another system library, the cross build links fine, and
# `cargo check` sees nothing. The glibc floor is untouched, yet the shipped .deb
# quietly stops working on a clean machine, because nothing in the pipeline
# compares what the ELF asks the
# loader for against what the package tells the package manager to install.
# Comparing those two lists is this script's whole job.
#
# It is the cheap companion to scripts/check-glibc-floor.sh: that one asks "is
# the glibc we need older than the floor", this one asks "is everything ELSE we
# need actually declared". Both run in the cross-build-nightly job, which is the
# only place a fully linked Linux binary exists.
#
# Usage:
#   scripts/check-linux-runtime-deps.sh [BIN_DIR]      # check, exit non-zero on drift
#   scripts/check-linux-runtime-deps.sh --print-deb-packages [BIN_DIR]
#   scripts/check-linux-runtime-deps.sh --print-rpm-packages [BIN_DIR]
# BIN_DIR defaults to target/x86_64-unknown-linux-gnu/release.
#
# The --print modes emit the space-separated package list derived from the ELF
# itself, so a caller that has to install those libraries (the nightly's boot
# test) never hardcodes a second copy of the list that could go stale.
set -euo pipefail
cd "$(dirname "$0")/.."
# shellcheck source=scripts/cross-env.sh
. "$(dirname "$0")/cross-env.sh"   # for $LINUX_RUST_IMG — the pinned bullseye image

PRINT_MODE=""
BIN_DIR="target/x86_64-unknown-linux-gnu/release"
for arg in "$@"; do
  case "$arg" in
    --print-deb-packages) PRINT_MODE=deb ;;
    --print-rpm-packages) PRINT_MODE=rpm ;;
    -*) echo "::error::unknown flag $arg" >&2; exit 2 ;;
    *)  BIN_DIR="$arg" ;;
  esac
done

for b in biorouterd biorouter biorouter-crew; do
  [ -f "$BIN_DIR/$b" ] || { echo "::error::missing binary $BIN_DIR/$b — run the cross build first"; exit 2; }
done

# Present on any glibc system that can run a Rust binary at all, so no package
# ever has to name them. ld-linux is the loader itself; libpthread/libdl/librt
# are stubs folded into libc since glibc 2.34 but still listed by binaries built
# against the 2.31 floor, which is exactly what we ship.
BASE_LIBS="ld-linux-x86-64.so.2 libc.so.6 libdl.so.2 libgcc_s.so.1 libm.so.6 libpthread.so.0 libresolv.so.2 librt.so.1 libutil.so.1"

# SONAME:debian-package:rpm-package. Everything the binaries need beyond the
# base set has to appear here AND in both packaging specs. Adding a row is a
# deliberate act: it means we have decided to ask users for another system
# library, so the row and the two spec edits belong in one commit.
PACKAGE_MAP="libxcb.so.1:libxcb1:libxcb
libz.so.1:zlib1g:zlib"

# readelf, not objdump: readelf parses any ELF regardless of the architecture it
# was built for, so this works when the pinned image resolves to arm64 (a
# developer's Mac) as well as on the amd64 CI runner. objdump is single-target
# and would answer "File format not recognized" on the first of those.
if ! dynamic=$(docker run --rm -v "$PWD":/w -w /w "$LINUX_RUST_IMG" sh -ec '
    for binary do
      readelf -d "$binary" || exit 2
    done
  ' sh "$BIN_DIR/biorouter" "$BIN_DIR/biorouterd" "$BIN_DIR/biorouter-crew"); then
  echo "::error::could not inspect every Linux binary's runtime dependencies" >&2
  exit 2
fi
needed=$(printf '%s\n' "$dynamic" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p' | sort -u)

[ -n "$needed" ] || { echo "::error::could not read DT_NEEDED entries from $BIN_DIR"; exit 2; }

deb_pkgs=""
rpm_pkgs=""
undeclared=""
for lib in $needed; do
  case " $BASE_LIBS " in *" $lib "*) continue ;; esac
  row=$(printf '%s\n' $PACKAGE_MAP | grep "^$lib:" || true)
  if [ -z "$row" ]; then
    undeclared="$undeclared $lib"
    continue
  fi
  deb_pkgs="$deb_pkgs $(printf '%s' "$row" | cut -d: -f2)"
  rpm_pkgs="$rpm_pkgs $(printf '%s' "$row" | cut -d: -f3)"
done

if [ -n "$undeclared" ]; then
  echo "::error::the shipped Linux binaries now hard-link libraries this repo has never declared:$undeclared"
  echo "  A new crate has added a system dependency. It will NOT show up as a build" >&2
  echo "  failure and it will NOT show up in the glibc floor check — it shows up as" >&2
  echo "  exit 127 on a user's clean machine. Decide the distro package names, then:" >&2
  echo "    1. add a SONAME:deb:rpm row to PACKAGE_MAP in this script" >&2
  echo "    2. add the deb name to overrides.deb.depends and the rpm name to" >&2
  echo "       overrides.rpm.depends in packaging/biorouter-cli.yaml" >&2
  echo "    3. add both to the maker-deb depends / maker-rpm requires arrays in" >&2
  echo "       ui/desktop/forge.config.ts (the desktop packages bundle these same" >&2
  echo "       two binaries)" >&2
  echo "  If the honest answer is that we do not want to ask users for that library," >&2
  echo "  the fix is upstream in the dependency, not here." >&2
  exit 1
fi

if [ -n "$PRINT_MODE" ]; then
  case "$PRINT_MODE" in
    deb) printf '%s\n' "${deb_pkgs# }" ;;
    rpm) printf '%s\n' "${rpm_pkgs# }" ;;
  esac
  exit 0
fi

# ── The other half: the packaging specs must actually name them ──────────────
# Extract one `depends:` list out of nfpm.yaml's `overrides:` block. nfpm's own
# schema is the only structure being relied on here (two-space override key,
# four-space list key, six-space items), so this stays readable without pulling
# a YAML parser into a CI guard.
nfpm_depends() {
  awk -v want="$1" '
    /^overrides:/            { in_ov = 1; next }
    in_ov && /^[^ ]/         { in_ov = 0 }
    in_ov && /^  [a-z]+:/    { sect = $1; sub(":", "", sect); key = ""; next }
    in_ov && /^    [a-z]+:/  { key  = $1; sub(":", "", key);  next }
    in_ov && /^      - / && sect == want && key == "depends" { print $2 }
  ' packaging/biorouter-cli.yaml
}

fail=0
err() { printf '::error::%s\n' "$1" >&2; fail=1; }

deb_declared=$(nfpm_depends deb)
rpm_declared=$(nfpm_depends rpm)
forge_config=$(tr '\n' ' ' < ui/desktop/forge.config.ts)

for p in $deb_pkgs; do
  printf '%s\n' "$deb_declared" | grep -qx "$p" \
    || err "packaging/biorouter-cli.yaml overrides.deb.depends is missing '$p' — the CLI .deb would install onto a clean Debian and then fail to start"
  # The desktop .deb bundles the same two binaries inside the Electron app. It
  # survives today only INCIDENTALLY: electron-installer-debian's own defaults
  # ask for libgtk-3-0, which drags libxcb1 in transitively. Declaring it makes
  # the requirement ours rather than a side effect of a dependency we do not
  # control — the arrays are merged with the Electron defaults (lodash union in
  # electron-installer-common), so naming it again is additive and safe.
  printf '%s\n' "$forge_config" | grep -qE "depends: \[[^]]*'$p'" \
    || err "ui/desktop/forge.config.ts maker-deb depends is missing '$p'"
done

for p in $rpm_pkgs; do
  printf '%s\n' "$rpm_declared" | grep -qx "$p" \
    || err "packaging/biorouter-cli.yaml overrides.rpm.depends is missing '$p' — the CLI .rpm would install onto a clean Rocky/RHEL and then fail to start"
  printf '%s\n' "$forge_config" | grep -qE "requires: \[[^]]*'$p'" \
    || err "ui/desktop/forge.config.ts maker-rpm requires is missing '$p'"
done

[ "$fail" = 0 ] || exit 1
echo "OK — beyond glibc the shipped binaries need:${deb_pkgs:-  (nothing)}, and every package declares it."
