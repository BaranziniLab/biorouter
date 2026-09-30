#!/usr/bin/env bash
set -euo pipefail

SOURCE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=scripts/release.sh
. "$SOURCE_ROOT/scripts/release.sh"

FIXTURE_ROOT="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_ROOT"' EXIT
ROOT="$FIXTURE_ROOT/repo"
DESK="$ROOT/ui/desktop"
RELEASE_REPOSITORY="example/biorouter"
VERSION=1.89.2

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

expect_failure() {
  local description="$1"
  shift
  if ( "$@" >/dev/null 2>&1 ); then
    fail "$description unexpectedly succeeded"
  fi
}

# release.sh normally runs from ROOT. This fixture changes ROOT after sourcing,
# so keep version lookup explicitly attached to the isolated test repository.
current_version() {
  perl -0ne 'print $1 if /\[workspace\.package\].*?version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/s' "$ROOT/Cargo.toml"
}

mkdir -p "$ROOT/ui/desktop"
git init -q -b main "$ROOT"
printf '[workspace.package]\nversion = "%s"\n' "$VERSION" >"$ROOT/Cargo.toml"
printf '/dist/\n/ui/desktop/out/\n' >"$ROOT/.gitignore"
git -C "$ROOT" -c user.name=Release-Test -c user.email=release-test@example.invalid \
  add Cargo.toml .gitignore
git -C "$ROOT" -c user.name=Release-Test -c user.email=release-test@example.invalid \
  commit -q -m fixture
SOURCE_SHA="$(git -C "$ROOT" rev-parse HEAD)"

(
  MAC_TRACE="$FIXTURE_ROOT/mac-backends.trace"
  activate_hermit() { :; }
  cargo() { printf 'cargo %s\n' "$*" >>"$MAC_TRACE"; }
  build_computer_use_helper() { printf 'helper %s\n' "$1" >>"$MAC_TRACE"; }
  ensure_docker() { fail "mac-backends unexpectedly requested Docker"; }
  run_cross_release() { fail "mac-backends unexpectedly cross-compiled a CI target"; }
  cmd_mac-backends "$VERSION"
  expected="$(printf '%s\n' 'cargo build --release' 'cargo build --release --target x86_64-apple-darwin' 'helper darwin-arm64' 'helper darwin-x64')"
  [ "$(cat "$MAC_TRACE")" = "$expected" ] || fail "mac-backends did not build both architectures and helpers"
  assert_release_source "$VERSION"
  mutating_mac_build() {
    cargo() { printf 'source changed\n' >"$ROOT/changed-during-build.txt"; }
    cmd_mac-backends "$VERSION"
  }
  expect_failure "source changed during mac backend build" mutating_mac_build
  rm -f "$ROOT/changed-during-build.txt"
)

# Plain fixture bytes exercise provenance, not archive extraction or signing.
# The production recorder obtains these rows from verify-computer-use-artifact.py.
record_fixture_asset() {
  local asset="$1" rel="${1#"$ROOT"/}"
  record_release_asset "$VERSION" "$asset"
  case "$asset" in
    *.yml|*.exe) ;;
    *) printf 'computer_use\t%s\t{"fixture":true}\n' "$rel" >>"$(release_provenance_file "$VERSION")" ;;
  esac
}

start_release_provenance "$VERSION"
while IFS= read -r asset; do
  mkdir -p "$(dirname "$asset")"
  printf 'fixture bytes for %s\n' "$(basename "$asset")" >"$asset"
  record_fixture_asset "$asset"
done < <(release_assets "$VERSION")
verify_release_provenance "$VERSION"

FIRST_ASSET="$(release_assets "$VERSION" | head -1)"
printf 'changed\n' >>"$FIRST_ASSET"
expect_failure "changed local asset provenance check" verify_release_provenance "$VERSION"
record_fixture_asset "$FIRST_ASSET"
verify_release_provenance "$VERSION"

fixture_manifest="$(release_provenance_file "$VERSION")"
awk -F '\t' -v rel="${FIRST_ASSET#"$ROOT"/}" '!($1 == "computer_use" && $2 == rel)' "$fixture_manifest" >"$FIXTURE_ROOT/missing-helper.tsv"
mv "$FIXTURE_ROOT/missing-helper.tsv" "$fixture_manifest"
expect_failure "missing helper attestation check" verify_release_provenance "$VERSION"
record_fixture_asset "$FIRST_ASSET"
verify_release_provenance "$VERSION"

printf 'source moved\n' >"$ROOT/source-moved.txt"
git -C "$ROOT" add source-moved.txt
git -C "$ROOT" -c user.name=Release-Test -c user.email=release-test@example.invalid \
  commit -q -m source-moved
expect_failure "source SHA provenance check" verify_release_provenance "$VERSION"
git -C "$ROOT" checkout -q --detach "$SOURCE_SHA"
verify_release_provenance "$VERSION"

ORIGIN="$FIXTURE_ROOT/origin.git"
git init -q --bare "$ORIGIN"
git -C "$ROOT" remote add origin "$ORIGIN"
git -C "$ROOT" push -q origin "$SOURCE_SHA:refs/heads/main"
require_remote_main_exact "$VERSION"

printf 'local ahead\n' >"$ROOT/local-ahead.txt"
git -C "$ROOT" add local-ahead.txt
git -C "$ROOT" -c user.name=Release-Test -c user.email=release-test@example.invalid \
  commit -q -m local-ahead
expect_failure "exact HEAD versus origin/main check" require_remote_main_exact "$VERSION"
git -C "$ROOT" checkout -q --detach "$SOURCE_SHA"
git -C "$ROOT" remote set-url origin "$FIXTURE_ROOT/missing-origin.git"
expect_failure "failed fetch check" require_remote_main_exact "$VERSION"
git -C "$ROOT" remote set-url origin "$ORIGIN"
require_remote_main_exact "$VERSION"

MANIFEST="$(release_provenance_file "$VERSION")"
RELEASES_JSON="$FIXTURE_ROOT/releases.json"
BAD_RELEASES_JSON="$FIXTURE_ROOT/releases-bad.json"
python3 - "$MANIFEST" "$VERSION" "$RELEASES_JSON" "$BAD_RELEASES_JSON" <<'PY'
import json
import os
import sys

manifest_path, version, output_path, bad_output_path = sys.argv[1:]
assets = []
with open(manifest_path, encoding="utf-8") as handle:
    for line in handle:
        fields = line.rstrip("\n").split("\t")
        if fields[0] == "asset":
            assets.append({
                "name": os.path.basename(fields[1]),
                "size": int(fields[3]),
                "digest": f"sha256:{fields[2]}",
                "updated_at": "2026-08-21T10:00:00Z",
            })
release = [{"tag_name": f"v{version}", "draft": True, "assets": assets}]
with open(output_path, "w", encoding="utf-8") as handle:
    json.dump(release, handle)
release[0]["assets"][0]["digest"] = "sha256:" + ("0" * 64)
with open(bad_output_path, "w", encoding="utf-8") as handle:
    json.dump(release, handle)
PY

RUNS_JSON="$FIXTURE_ROOT/runs-stale.json"
printf '[{"displayTitle":"Release artifact smoke v%s","conclusion":"success","startedAt":"2026-08-21T09:59:59Z","headSha":"%s","url":"https://example.invalid/stale"}]\n' \
  "$VERSION" "$SOURCE_SHA" >"$RUNS_JSON"

gh() {
  if [ "${1:-}" = api ]; then
    command cat "$RELEASES_JSON"
  elif [ "${1:-}" = run ] && [ "${2:-}" = list ]; then
    command cat "$RUNS_JSON"
  else
    return 2
  fi
}

verify_remote_release_assets "$VERSION"
[ "$LATEST_DRAFT_ASSET_UPDATED_AT" = "2026-08-21T10:00:00Z" ] \
  || fail "latest upload timestamp was not retained"
expect_failure "stale Windows smoke check" \
  require_fresh_windows_smoke "$VERSION" "$LATEST_DRAFT_ASSET_UPDATED_AT"

printf '[{"displayTitle":"Release artifact smoke v%s","conclusion":"success","startedAt":"2026-08-21T10:00:01Z","headSha":"%s","url":"https://example.invalid/fresh"}]\n' \
  "$VERSION" "$SOURCE_SHA" >"$RUNS_JSON"
require_fresh_windows_smoke "$VERSION" "$LATEST_DRAFT_ASSET_UPDATED_AT"

RELEASES_JSON="$BAD_RELEASES_JSON"
expect_failure "uploaded digest mismatch check" verify_remote_release_assets "$VERSION"

printf 'release hardening tests passed\n'
