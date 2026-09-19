#!/usr/bin/env bash
#
# Assert that every version string in the repo agrees with the single source of
# truth — the Cargo workspace version in `Cargo.toml` (`[workspace.package]`).
#
# Why this exists: the CLI (`biorouter`), the daemon (`biorouterd`), and the
# core library all inherit `version.workspace = true`, so the three Rust
# binaries can NEVER disagree at build time. The desktop GUI, however, carries
# its own version in `ui/desktop/package.json` (plus two copies in
# `package-lock.json` and one in `openapi.json`). `scripts/release.sh bump`
# rewrites all of them together, but nothing stops a hand edit from drifting one
# out of sync — which is exactly how you ship a GUI that reports a different
# version than its bundled daemon. This check is the guard; it runs in
# `just check-everything` and should run in CI.
#
# Exit non-zero (with a diff) on any mismatch, or if a Rust crate hardcodes its
# own version instead of inheriting the workspace one.

set -euo pipefail
cd "$(dirname "$0")/.."

fail=0
note() { printf '  %s\n' "$1"; }
err()  { printf '  ✗ %s\n' "$1"; fail=1; }

# ── source of truth ──────────────────────────────────────────────────────────
# First `version = "x.y.z"` under [workspace.package].
truth=$(awk '
  /^\[workspace\.package\]/ {inblock=1; next}
  /^\[/ {inblock=0}
  inblock && /^version[[:space:]]*=/ {
    gsub(/[^0-9A-Za-z.\-]/, "", $3); print $3; exit
  }' Cargo.toml)

if [[ -z "${truth:-}" ]]; then
  echo "✗ could not read workspace version from Cargo.toml [workspace.package]" >&2
  exit 2
fi
echo "Source of truth (Cargo [workspace.package].version): $truth"

# ── helper: extract a JSON value with python (already a build dep) ────────────
#
# ⚠ Resolve the interpreter; do not hardcode `python3`. On Windows `python3` is
# normally a 0-byte Microsoft Store app-execution alias that exits 9009 with
# "Python was not found", so `command -v python3` SUCCEEDS and running it does
# not. This script then read every desktop version as the empty string and
# reported drift against a tree that had none — a guard failing open into a
# false alarm, which is the kind people learn to ignore.
pick_python() {
  local candidate
  for candidate in python3 python py; do
    # Must both exist AND execute: existence alone is what the Store alias fakes.
    if command -v "$candidate" >/dev/null 2>&1 \
      && "$candidate" -c 'import json,sys' >/dev/null 2>&1; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  echo "✗ no working python interpreter found (tried python3, python, py)" >&2
  exit 2
}
PY="$(pick_python)"
jget() { "$PY" -c "import json,sys; print(json.load(open(sys.argv[1]))$2)" "$1"; }

# ── desktop JSON files ───────────────────────────────────────────────────────
check() {
  local label="$1" got="$2"
  if [[ "$got" == "$truth" ]]; then
    note "✓ $label = $got"
  else
    err "$label = $got  (expected $truth)"
  fi
}

check "ui/desktop/package.json .version"            "$(jget ui/desktop/package.json "['version']")"
check "ui/desktop/package-lock.json .version"       "$(jget ui/desktop/package-lock.json "['version']")"
check "ui/desktop/package-lock.json .packages[''].version" \
      "$(jget ui/desktop/package-lock.json "['packages']['']['version']")"
check "ui/desktop/openapi.json .info.version"       "$(jget ui/desktop/openapi.json "['info']['version']")"

# ── README badge ─────────────────────────────────────────────────────────────
# The version most people actually see, on the repo's front page. It was NOT
# covered here until 2026-08-04, and had silently drifted to 1.87.2 while the
# tree was on 1.88.6 — three releases stale on the first thing a visitor reads.
# `release.sh bump` now rewrites it; this is the guard that keeps it honest.
check "README.md badge URL" \
      "$(grep -oE 'badge/version-[0-9]+\.[0-9]+\.[0-9]+-tan\.svg' README.md | head -1 \
         | sed -E 's|badge/version-||; s|-tan\.svg||')"
check "README.md badge alt text" \
      "$(grep -oE 'alt="Version [0-9]+\.[0-9]+\.[0-9]+"' README.md | head -1 \
         | sed -E 's|alt="Version ||; s|"||')"

# ── every Rust crate must inherit the workspace version ──────────────────────
while IFS= read -r toml; do
  if ! grep -qE '^version\.workspace[[:space:]]*=[[:space:]]*true' "$toml"; then
    if grep -qE '^version[[:space:]]*=' "$toml"; then
      err "$toml hardcodes a version instead of 'version.workspace = true'"
    fi
  fi
done < <(find crates -maxdepth 2 -name Cargo.toml)

echo
if [[ "$fail" -eq 0 ]]; then
  echo "✅ All CLI / daemon / GUI version strings agree on $truth"
else
  echo "❌ Version drift detected. Fix with: scripts/release.sh bump $truth" >&2
  echo "   (or hand-edit the offending file to match $truth)" >&2
  exit 1
fi
