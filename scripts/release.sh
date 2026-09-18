#!/usr/bin/env bash
#
# Biorouter cross-platform release automation.
#
# Encodes the full release pipeline so a human OR an agent can cut a signed,
# notarized, multi-platform release reproducibly. Each phase is a separate
# subcommand so a workflow can run/verify them independently and resume.
#
#   scripts/release.sh <command> <version>
#
# Commands:
#   bump <ver>        Bump version in the 5 release files + refresh Cargo.lock.
#   backends <ver>    Compile release backends for all 4 targets
#                     (mac arm64, mac x64, windows-gnu, linux-gnu).
#   linux-backend <ver>
#                     Rebuild just the linux x86_64 backend from scratch.
#   mac-arm64 <ver>   Package + sign + NOTARIZE the Apple Silicon .dmg.
#   mac-intel <ver>   Package + sign + NOTARIZE the Intel .dmg.
#   windows <ver>     Package the Windows .zip.
#   linux <ver>       Package the GUI .deb + .rpm.
#   cli-linux <ver>   Build the CLI-only .deb + .rpm (no GUI).
#   mac-manifest <ver>
#                     Generate latest-mac.yml for electron-updater.
#   verify <ver>      Verify all release artifacts (arch, notarization, dmg format).
#   draft <ver>       Create a draft GitHub release with assets + notes.
#   publish <ver>     Publish a verified draft after native Windows smoke passes.
#   all <ver>         Run every build/verify phase and create the draft release.
#
# Hard-won invariants (see CLAUDE.md for the long version):
#   * The macOS .dmg maker (macos-alias native module) only builds under
#     Node 24 — use hermit's node, NOT a newer Homebrew node. All packaging
#     runs under `source bin/activate-hermit`.
#   * The windows-gnu / linux-gnu cross builds must run with the SYSTEM docker,
#     not hermit's docker shim (which points at the wrong socket). We invoke
#     `docker` directly here rather than via `just`.
#   * aws-lc-sys (AWS SDK / rustls) needs winpthread appended AFTER the rlibs on
#     the mingw link line; lzma-sys (xz2, .brkb path) needs LZMA_API_STATIC=1 so
#     it statically builds bundled liblzma instead of finding the host one.
#     Both are applied below for the cross targets (and live in the Justfile).
#   * Every bundle writes ui/desktop/src/bin/ and clobbers the others — phases
#     stage the correct binaries and run strictly one platform at a time.
#   * The Linux docker package runs `npm ci` and leaves node_modules Linux-
#     flavored. Host-side browser, macOS, and headless builds must restore the
#     native optional dependencies before they run.
#   * Notarization credentials are read from notarization/APPLE_DEVELOPER_NOTES.md
#     (gitignored). Override via APPLE_ID / APPLE_APP_SPECIFIC_PASSWORD env vars.
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
DESK="$ROOT/ui/desktop"
NOTARY="$ROOT/notarization"
SIGN_IDENTITY="Developer ID Application: University of California at San Francisco (F3YYBXAFJ8)"
TEAM_ID="F3YYBXAFJ8"
RELEASE_REPOSITORY="${BIOROUTER_RELEASE_REPOSITORY:-BaranziniLab/biorouter}"
RELEASE_PROVENANCE_SCHEMA=1

# The docker cross-compile recipes (linux/windows images, mingw winpthread
# linker wrap, LZMA_API_STATIC, the glibc-2.31 pin) live in ONE place so the
# release and the BR-70 `check-cross` CI gate can never drift apart.
# shellcheck source=scripts/cross-env.sh
. "$ROOT/scripts/cross-env.sh"

log()  { printf '\033[1;36m[release]\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31m[release] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

need_version() { [ -n "${1:-}" ] || die "version required, e.g. scripts/release.sh $CMD 1.80.1"; }

activate_hermit() { set +u; source "$ROOT/bin/activate-hermit" >/dev/null 2>&1 || true; set -u; }

# Resolve notarization creds: env → macOS Keychain → gitignored notes file.
# The Keychain is the preferred store (encrypted at rest, no plaintext on disk).
# Seed it once (any local build agent can then read it without a prompt):
#   security add-generic-password -s biorouter-notarization -a APPLE_ID -w <id> -A -U
#   security add-generic-password -s biorouter-notarization -a APPLE_APP_SPECIFIC_PASSWORD -w <pw> -A -U
load_apple_creds() {
  if [ -z "${APPLE_ID:-}" ]; then
    APPLE_ID="$(security find-generic-password -s biorouter-notarization -a APPLE_ID -w 2>/dev/null || true)"
  fi
  if [ -z "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]; then
    APPLE_APP_SPECIFIC_PASSWORD="$(security find-generic-password -s biorouter-notarization -a APPLE_APP_SPECIFIC_PASSWORD -w 2>/dev/null || true)"
  fi
  if [ -z "${APPLE_ID:-}" ] || [ -z "${APPLE_APP_SPECIFIC_PASSWORD:-}" ]; then
    [ -f "$NOTARY/APPLE_DEVELOPER_NOTES.md" ] || die "no APPLE_ID env, no Keychain item, and no $NOTARY/APPLE_DEVELOPER_NOTES.md"
    APPLE_ID="$(grep -iE 'Apple ID \(notarization\)' "$NOTARY/APPLE_DEVELOPER_NOTES.md" | grep -oE '`[^`]+`' | tr -d '`' | head -1)"
    APPLE_APP_SPECIFIC_PASSWORD="$(grep -iE 'App-specific password' "$NOTARY/APPLE_DEVELOPER_NOTES.md" | grep -oE '`[^`]+`' | tr -d '`' | head -1)"
  fi
  [ -n "$APPLE_ID" ] && [ -n "$APPLE_APP_SPECIFIC_PASSWORD" ] || die "could not resolve Apple notarization credentials"
  export APPLE_ID APPLE_APP_SPECIFIC_PASSWORD
}

ensure_docker() {
  docker info >/dev/null 2>&1 && return 0
  log "starting Docker Desktop…"; open -a Docker || true
  for _ in $(seq 1 60); do docker info >/dev/null 2>&1 && return 0; sleep 5; done
  die "Docker daemon not reachable"
}

# The macOS .dmg maker needs the darwin-only `appdmg` (+ its native deps
# macos-alias / ds-store). A prior Linux docker package or a partial reinstall
# can drop them, and they must be (re)built against hermit's Node. Ensure they
# are present and loadable before any dmg build, otherwise the maker dies with
# "Cannot find module 'appdmg'" / a NODE_MODULE_VERSION mismatch.
ensure_mac_dmg_deps() {
  ( cd "$DESK"
    log "installing locked macOS desktop dependencies…"
    # ⚠ The install output is KEPT. It used to be `npm ci >/dev/null 2>&1`, and a
    # failing install was then invisible: the run carried on to the `require`
    # below and died with "appdmg still not loadable", which names the symptom
    # and hides the cause. A 1.89.5 phase failed exactly that way and the reason
    # was unrecoverable after the fact, because the only evidence had been sent
    # to /dev/null. Whatever npm says on failure is the thing worth having.
    local out
    if ! out="$(npm ci 2>&1)"; then
      printf '%s\n' "$out" >&2
      die "npm ci failed while installing macOS desktop dependencies (output above)"
    fi
    npm rebuild macos-alias ds-store >/dev/null 2>&1 || true
    # appdmg is an OPTIONAL dependency of electron-installer-dmg, and npm skips an
    # optional dep whose install fails rather than failing the install. So a green
    # `npm ci` does not imply appdmg is present, and this stays a separate check.
    node -e "require('appdmg')" >/dev/null 2>&1 \
      || die "appdmg is not loadable after a successful npm ci. It is an optional
dependency, so npm skipped it silently rather than failing. Check that this is
hermit's npm (npm $(npm --version), node $(node --version)) and that install
scripts are permitted -- a newer npm blocks native install scripts by default,
which drops appdmg without an error."
  )
}

ensure_host_node_deps() {
  activate_hermit
  (
    cd "$DESK"
    log "installing locked host-native desktop dependencies…"
    npm ci >/dev/null
    node -e "require('rollup')" >/dev/null 2>&1 \
      || die "host-native Rollup dependency is unavailable"
  )
}

# The release phases are intentionally resumable, which also means a later phase
# can otherwise consume artifacts built from a different checkout. Keep a local,
# ignored manifest that binds every uploaded byte to the clean source commit from
# which the backend build started. The manifest is not itself a release asset.
release_provenance_file() {
  printf '%s/dist/release-build-%s.tsv\n' "$ROOT" "$1"
}

release_file_sha256() {
  shasum -a 256 "$1" | awk '{print $1}'
}

release_file_size() {
  if stat -f %z "$1" >/dev/null 2>&1; then
    stat -f %z "$1"
  else
    stat -c %s "$1"
  fi
}

require_clean_release_tree() {
  local dirty
  dirty="$(git -C "$ROOT" status --porcelain --untracked-files=normal)"
  [ -z "$dirty" ] || die "release source tree is not clean; commit or remove source changes before building:\n$dirty"
}

release_provenance_value() {
  local file="$1" key="$2"
  awk -F '\t' -v key="$key" '
    $1 == key { value=$2; count++ }
    END { if (count != 1) exit 1; print value }
  ' "$file"
}

start_release_provenance() {
  local v="$1" file tmp source_sha
  [ "$(current_version)" = "$v" ] \
    || die "Cargo.toml version $(current_version) does not match release $v"
  require_clean_release_tree
  source_sha="$(git -C "$ROOT" rev-parse HEAD)"
  file="$(release_provenance_file "$v")"
  mkdir -p "$(dirname "$file")"
  tmp="$(mktemp "${file}.XXXXXX")"
  {
    printf 'schema\t%s\n' "$RELEASE_PROVENANCE_SCHEMA"
    printf 'version\t%s\n' "$v"
    printf 'source_sha\t%s\n' "$source_sha"
    printf 'created_at\t%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  } >"$tmp"
  mv "$tmp" "$file"
  log "release provenance started at $source_sha"
}

assert_release_source() {
  local v="$1" file source_sha
  file="$(release_provenance_file "$v")"
  [ -f "$file" ] || die "release provenance missing: run scripts/release.sh backends $v before packaging"
  [ "$(release_provenance_value "$file" schema)" = "$RELEASE_PROVENANCE_SCHEMA" ] \
    || die "unsupported or corrupt release provenance: $file"
  [ "$(release_provenance_value "$file" version)" = "$v" ] \
    || die "release provenance version does not match $v"
  [ "$(current_version)" = "$v" ] \
    || die "Cargo.toml version $(current_version) does not match release $v"
  source_sha="$(release_provenance_value "$file" source_sha)"
  [ "$(git -C "$ROOT" rev-parse HEAD)" = "$source_sha" ] \
    || die "HEAD changed after release builds started; rebuild all artifacts from the final source commit"
  require_clean_release_tree
}

record_release_asset() {
  local v="$1" file="$2" manifest rel digest size tmp runtime_target="${3:-}" runtime_evidence=""
  if [ -n "$runtime_target" ]; then
    case "$runtime_target" in
      darwin-*) runtime_evidence="$(python3 "$ROOT/scripts/verify-computer-use-artifact.py" "$file" "$runtime_target" --require-signed)" ;;
      *) runtime_evidence="$(python3 "$ROOT/scripts/verify-computer-use-artifact.py" "$file" "$runtime_target")" ;;
    esac
  fi
  assert_release_source "$v"
  [ -f "$file" ] || die "release artifact missing: $file"
  case "$file" in
    "$ROOT"/*) rel="${file#"$ROOT"/}" ;;
    *) die "release artifact is outside the repository: $file" ;;
  esac
  manifest="$(release_provenance_file "$v")"
  digest="$(release_file_sha256 "$file")"
  size="$(release_file_size "$file")"
  tmp="$(mktemp "${manifest}.XXXXXX")"
  awk -F '\t' -v rel="$rel" '!( ($1 == "asset" || $1 == "computer_use") && $2 == rel )' "$manifest" >"$tmp"
  printf 'asset\t%s\t%s\t%s\n' "$rel" "$digest" "$size" >>"$tmp"
  if [ -n "$runtime_evidence" ]; then
    printf 'computer_use\t%s\t%s\n' "$rel" "$runtime_evidence" >>"$tmp"
  fi
  mv "$tmp" "$manifest"
  log "recorded release provenance: $(basename "$file")"
}

verify_release_provenance() {
  local v="$1" manifest file rel expected entry count digest size expected_count=0 actual_count
  assert_release_source "$v"
  manifest="$(release_provenance_file "$v")"
  while IFS= read -r file; do
    expected_count=$((expected_count + 1))
    rel="${file#"$ROOT"/}"
    count="$(awk -F '\t' -v rel="$rel" '$1 == "asset" && $2 == rel { count++ } END { print count+0 }' "$manifest")"
    [ "$count" -eq 1 ] || die "release provenance must contain exactly one entry for $rel"
    case "$file" in
      *.yml) ;;
      *)
        count="$(awk -F '\t' -v rel="$rel" '$1 == "computer_use" && $2 == rel { count++ } END { print count+0 }' "$manifest")"
        [ "$count" -eq 1 ] || die "release provenance must attest helper bytes exactly once for $rel"
        ;;
    esac
    entry="$(awk -F '\t' -v rel="$rel" '$1 == "asset" && $2 == rel { print $3 "\t" $4 }' "$manifest")"
    IFS=$'\t' read -r digest size <<<"$entry"
    [ -f "$file" ] || die "release artifact missing: $file"
    expected="$(release_file_sha256 "$file")"
    [ "$digest" = "$expected" ] || die "release artifact changed after build: $rel"
    expected="$(release_file_size "$file")"
    [ "$size" = "$expected" ] || die "release artifact size changed after build: $rel"
  done < <(release_assets "$v")
  actual_count="$(awk -F '\t' '$1 == "asset" { count++ } END { print count+0 }' "$manifest")"
  [ "$actual_count" -eq "$expected_count" ] \
    || die "release provenance contains $actual_count assets; expected exactly $expected_count"
  [ "$expected_count" -eq 10 ] || die "internal release asset list changed; expected exactly 10 assets"
  local helper_count
  helper_count="$(awk -F '\t' '$1 == "computer_use" { count++ } END { print count+0 }' "$manifest")"
  [ "$helper_count" -eq 9 ] || die "release provenance must attest the helper bytes in all 9 install/update archives"
  log "release provenance verified for 10 assets at $(release_provenance_value "$manifest" source_sha)"
}

# ── bump ────────────────────────────────────────────────────────────────────
cmd_bump() {
  local v="$1"
  log "bumping version → $v"
  # Cargo workspace package version (the line under [workspace.package]).
  perl -0pi -e "s/(\[workspace\.package\][^\[]*?version = \")[0-9.]+(\")/\${1}$v\${2}/s" Cargo.toml
  # The three desktop JSON files (package.json, package-lock.json x2, openapi.json).
  python3 - "$v" <<'PY'
import json, sys
v = sys.argv[1]
def setver(path, fn):
    with open(path, encoding='utf-8') as f: data = json.load(f)
    fn(data)
    # ensure_ascii=False: these files contain literal em-dashes/ellipses (openapi.json
    # doc comments). Escaping them to \uXXXX churns hundreds of unrelated lines.
    # encoding pinned so a C/POSIX locale can't turn that into a UnicodeEncodeError.
    with open(path, 'w', encoding='utf-8') as f: json.dump(data, f, indent=2, ensure_ascii=False); f.write('\n')
setver('ui/desktop/package.json', lambda d: d.__setitem__('version', v))
setver('ui/desktop/openapi.json', lambda d: d['info'].__setitem__('version', v))
def lock(d):
    d['version'] = v
    d['packages'][''] ['version'] = v
setver('ui/desktop/package-lock.json', lock)
PY
  # README badge. Not one of the five for a long time, which is exactly why it
  # drifted to 1.87.2 while the tree was on 1.88.6 — it is the version most
  # people actually see, on the repo's front page, and nothing moved it.
  # `check-version-consistency.sh` now fails if this and Cargo.toml disagree.
  perl -0pi -e "s{(badge/version-)[0-9]+\.[0-9]+\.[0-9]+(-tan\.svg)}{\${1}$v\${2}}g" README.md
  perl -0pi -e "s{(alt=\"Version )[0-9]+\.[0-9]+\.[0-9]+(\")}{\${1}$v\${2}}g" README.md
  activate_hermit
  cargo update -p biorouter --precise "$v" >/dev/null 2>&1 || cargo check -q >/dev/null 2>&1 || true
  log "version is now: $(grep -m1 '^version' Cargo.toml)"
}

# ── backends ──────────────────────────────────────────────────────────────────
# The cross-compile images, toolchain env, mingw linker wrap and LZMA_API_STATIC
# now live in scripts/cross-env.sh (sourced above) — the SAME recipe the BR-70
# `check-cross` CI gate uses, so what the gate checks is exactly what ships.

# Linux x86_64 backend (biorouterd + biorouter). Extracted so it can be re-run
# on its own. Cleans the target dir first to force a from-scratch compile
# against the pinned glibc (cached objects would keep stale symbol versions).
cmd_linux-backend() {
  local v="$1"
  assert_release_source "$v"
  ensure_docker
  log "cross-compiling linux-gnu backend (docker, $LINUX_RUST_IMG)"
  rm -rf "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter" \
         "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouterd"
  run_cross_release \
    cross_linux \
    biorouter-linux-release-target \
    "cargo build --release --bin biorouterd --bin biorouter" \
    "mkdir -p /usr/src/myapp/target/x86_64-unknown-linux-gnu/release && \
     cp -f /cross-target/x86_64-unknown-linux-gnu/release/biorouter \
           /cross-target/x86_64-unknown-linux-gnu/release/biorouterd \
           /usr/src/myapp/target/x86_64-unknown-linux-gnu/release/"
  assert_glibc_floor
  log "linux backend compiled"
}

# The Linux baseline is Debian 11 / Ubuntu 22.04 / RHEL-Rocky 9, i.e. glibc 2.31.
#
# ⚠ NOTHING ELSE ENFORCES THIS, despite CLAUDE.md having said the cli-linux smoke
# test does. It cannot: those containers are `debian:bookworm` (glibc 2.36) and
# `rockylinux:9` (glibc 2.34), so a floor raised from 2.31 to 2.34 passes every
# smoke test in this repo and breaks only on the user's older machine. Measured
# 2026-08-20. The check is a symbol-table read, so it needs no container and runs
# in milliseconds.
GLIBC_MAX="2.31"
assert_glibc_floor() {
  local bin found=""
  for bin in "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouterd" \
             "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter"; do
    [ -f "$bin" ] || continue
    local worst
    worst="$(strings -a "$bin" 2>/dev/null | grep -oE 'GLIBC_2\.[0-9]+' \
             | sort -t. -k2 -n | tail -1)"
    [ -n "$worst" ] || continue
    found="yes"
    local want="${worst#GLIBC_}"
    # numeric compare on the minor, since 2.9 must not sort above 2.31
    if [ "${want#2.}" -gt "${GLIBC_MAX#2.}" ]; then
      die "$(basename "$bin") needs $worst but the Linux baseline is GLIBC_$GLIBC_MAX (Debian 11 / Ubuntu 22.04 / Rocky 9). The cross image pin (LINUX_RUST_IMG) has probably moved. No smoke test in this repo can catch this — they all run on newer glibc."
    fi
  done
  [ -n "$found" ] || die "could not read a GLIBC symbol from either linux binary — the floor was NOT checked"
  log "glibc floor ok (<= GLIBC_$GLIBC_MAX)"
}

run_cross_release() { # <cross function> <target volume> <cargo command> <post command>
  local cross_fn="$1" target_volume="$2" cargo_cmd="$3" post_cmd="$4" rc=0
  docker volume rm -f "$target_volume" >/dev/null 2>&1 || true
  docker volume create "$target_volume" >/dev/null
  (
    export CROSS_TARGET_MOUNT="$target_volume"
    "$cross_fn" "$cargo_cmd" /cross-target "$post_cmd"
  ) || rc=$?
  docker volume rm -f "$target_volume" >/dev/null 2>&1 || true
  return "$rc"
}

cmd_backends() {
  local v="$1"
  start_release_provenance "$v"
  activate_hermit
  log "compiling mac arm64 release backend"
  cargo build --release
  log "compiling mac x64 release backend"
  cargo build --release --target x86_64-apple-darwin

  ensure_docker
  log "cross-compiling windows-gnu backend (docker)"
  run_cross_release \
    cross_windows \
    biorouter-windows-release-target \
    "cargo build --release --bin biorouterd --bin biorouter" \
    "mkdir -p /usr/src/myapp/target/x86_64-pc-windows-gnu/release && \
     cp -f /cross-target/x86_64-pc-windows-gnu/release/biorouter.exe \
           /cross-target/x86_64-pc-windows-gnu/release/biorouterd.exe \
           /usr/src/myapp/target/x86_64-pc-windows-gnu/release/ && \
     $WIN_DLL_STAGE"

  cmd_linux-backend "$v"
  for helper_target in darwin-arm64 darwin-x64 win32-x64 linux-x64; do
    python3 "$ROOT/scripts/computer-use-runtime.py" build "$helper_target" --signing-identity "$SIGN_IDENTITY"
  done
  assert_release_source "$v"
  log "all 4 backends compiled"
}

stage_bin() { # <src-dir> <ext>
  rm -rf "$DESK/src/bin"; mkdir -p "$DESK/src/bin"
  cp -p "$1/biorouter${2:-}" "$1/biorouterd${2:-}" "$DESK/src/bin/"
}

# ── mac packaging (sign + notarize, Node 24 via hermit) ───────────────────────
cmd_mac-arm64() {
  local v="$1"; assert_release_source "$v"; activate_hermit; load_apple_creds; ensure_mac_dmg_deps
  ls /Volumes/Biorouter* >/dev/null 2>&1 && { umount /Volumes/Biorouter* 2>/dev/null || true; }
  python3 "$ROOT/scripts/computer-use-runtime.py" verify darwin-arm64 --require-signed
  stage_bin "$ROOT/target/release"
  log "building + notarizing macOS arm64 dmg"
  ( cd "$DESK" && APPLE_ID="$APPLE_ID" APPLE_APP_SPECIFIC_PASSWORD="$APPLE_APP_SPECIFIC_PASSWORD" npm run bundle:default )
  # ⚠ Assert, do not announce. `bundle:default` used to end in `|| echo …`, which
  # turned every failure into exit 0, so this phase could not fail and printed a
  # path for a dmg it had not built. The `||` is gone from package.json now; this
  # check is the belt to that braces, and mirrors what cmd_linux already does.
  local dmg="$DESK/out/make/Biorouter-$v-arm64.dmg"
  local updater_zip="$DESK/out/make/$ARM64_ZIP_REL/Biorouter-darwin-arm64-$v.zip"
  [ -f "$dmg" ] || die "mac-arm64 reported success but produced no dmg at $dmg"
  record_release_asset "$v" "$dmg" "darwin-arm64"
  record_release_asset "$v" "$updater_zip" "darwin-arm64"
  log "arm64 dmg: $dmg"
}

cmd_mac-intel() {
  local v="$1"; assert_release_source "$v"; activate_hermit; load_apple_creds; ensure_mac_dmg_deps
  ls /Volumes/Biorouter* >/dev/null 2>&1 && { umount /Volumes/Biorouter* 2>/dev/null || true; }
  python3 "$ROOT/scripts/computer-use-runtime.py" verify darwin-x64 --require-signed
  stage_bin "$ROOT/target/x86_64-apple-darwin/release"
  log "building + notarizing macOS Intel dmg"
  ( cd "$DESK" && APPLE_ID="$APPLE_ID" APPLE_APP_SPECIFIC_PASSWORD="$APPLE_APP_SPECIFIC_PASSWORD" npm run bundle:intel )
  local dmg="$DESK/out/make/Biorouter-$v-x64.dmg"
  local updater_zip="$DESK/out/make/$X64_ZIP_REL/Biorouter-darwin-x64-$v.zip"
  [ -f "$dmg" ] || die "mac-intel reported success but produced no dmg at $dmg"
  record_release_asset "$v" "$dmg" "darwin-x64"
  record_release_asset "$v" "$updater_zip" "darwin-x64"
  log "x64 dmg: $dmg"
}

# ── windows packaging (host forge, Node 24) ───────────────────────────────────
cmd_windows() {
  local v="$1"; assert_release_source "$v"; activate_hermit; ensure_host_node_deps
  local WR="$ROOT/target/x86_64-pc-windows-gnu/release"
  [ -f "$WR/biorouterd.exe" ] || die "windows backend missing — run: scripts/release.sh backends $v"
  rm -rf "$DESK/src/bin"; mkdir -p "$DESK/src/bin"
  cp -f "$WR/biorouterd.exe" "$WR/biorouter.exe" "$WR"/*.dll "$DESK/src/bin/"
  log "packaging Windows zip"
  ( cd "$DESK" && npm run bundle:windows )
  local zip="$DESK/out/make/zip/win32/x64/Biorouter-win32-x64-$v.zip"
  [ -f "$zip" ] || die "windows reported success but produced no zip at $zip"
  record_release_asset "$v" "$zip" "win32-x64"
  log "windows zip: $zip"
}

# ── linux packaging (fully dockerized; run LAST — corrupts node_modules) ───────
cmd_linux() {
  local v="$1"; assert_release_source "$v"; ensure_docker
  [ -f "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouterd" ] || die "linux backend missing — run: scripts/release.sh backends $v"
  log "packaging Linux deb + rpm (docker)"
  docker volume create biorouter-linux-npm-cache >/dev/null 2>&1 || true
  docker run --rm --platform linux/amd64 -v "$ROOT":/ws -v biorouter-linux-npm-cache:/root/.npm \
    "$(node -e 'console.log(require(process.argv[1]).image)' "$ROOT/ui/desktop/scripts/linux-native-baseline.json")" bash /ws/ui/desktop/scripts/build-linux-deb.sh \
    || die "the linux docker build failed — see the output above"
  # Assert the artifacts EXIST before announcing them. A bash function returns
  # its LAST command's status, so the three `log` lines that used to end this
  # function masked a failing `docker run` entirely: the phase exited 0, a
  # `set -e` chain sailed past it, and it printed these two paths for files it
  # had not produced. That happened — `npm ci` refused an out-of-sync lock
  # inside the container and the release still reported the phase OK.
  local deb="$DESK/out/make/deb/x64/biorouter_${v}_amd64.deb"
  local rpm="$DESK/out/make/rpm/x64/Biorouter-$v-1.x86_64.rpm"
  [ -f "$deb" ] || die "linux phase reported success but produced no deb at $deb"
  [ -f "$rpm" ] || die "linux phase reported success but produced no rpm at $rpm"
  record_release_asset "$v" "$deb" "linux-x64"
  record_release_asset "$v" "$rpm" "linux-x64"
  log "deb: $deb"
  log "rpm: $rpm"
  # `npm ci`, NOT `npm install`: install rewrites package-lock.json, and the
  # container's `npm ci` then refuses the out-of-sync lock on the next run.
  log "NOTE: node_modules is now Linux-flavored — run 'cd ui/desktop && npm ci' before any further mac build."
}

# ── CLI-only Linux packages (deb + rpm; headless biorouter + biorouterd) ───────
# Independent of the GUI packaging — does NOT corrupt node_modules. Builds and
# smoke-tests both packages in clean containers.
cmd_cli-linux() {
  # ensure_host_node_deps because these packages now carry the browser interface
  # bundle, which is built on the HOST by `npm run build:web`. After a Linux or
  # Windows docker build the on-disk node_modules is Linux-flavored and that
  # build dies on a missing @rollup/rollup-darwin-arm64 — a failure that reads
  # as a rollup bug rather than an ordering one. The retired headless phase had
  # this call for the same reason; it is needed here now that the payload moved.
  local v="$1"; assert_release_source "$v"; ensure_docker; ensure_host_node_deps
  [ -f "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter" ] || die "linux backend missing — run: scripts/release.sh backends $v"
  log "building CLI-only Linux packages (deb + rpm)"
  bash "$ROOT/scripts/build-cli-linux-packages.sh" "$v"
  record_release_asset "$v" "$ROOT/dist/cli/biorouter-cli_${v}_amd64.deb" "linux-x64"
  record_release_asset "$v" "$ROOT/dist/cli/biorouter-cli-${v}-1.x86_64.rpm" "linux-x64"
  log "cli deb: $ROOT/dist/cli/biorouter-cli_${v}_amd64.deb"
  log "cli rpm: $ROOT/dist/cli/biorouter-cli-${v}-1.x86_64.rpm"
}

# ── verify ────────────────────────────────────────────────────────────────────
cmd_verify() {
  local v="$1" ok=1
  assert_release_source "$v"
  "$ROOT/scripts/check-brand-consistency.sh"
  local arm="$DESK/out/make/Biorouter-$v-arm64.dmg"
  local x64="$DESK/out/make/Biorouter-$v-x64.dmg"
  local win="$DESK/out/make/zip/win32/x64/Biorouter-win32-x64-$v.zip"
  local deb="$DESK/out/make/deb/x64/biorouter_${v}_amd64.deb"
  local rpm="$DESK/out/make/rpm/x64/Biorouter-$v-1.x86_64.rpm"
  local clideb="$ROOT/dist/cli/biorouter-cli_${v}_amd64.deb"
  local clirpm="$ROOT/dist/cli/biorouter-cli-${v}-1.x86_64.rpm"
  local armzip="$DESK/out/make/$ARM64_ZIP_REL/Biorouter-darwin-arm64-$v.zip"
  local x64zip="$DESK/out/make/$X64_ZIP_REL/Biorouter-darwin-x64-$v.zip"
  for f in "$arm" "$x64" "$armzip" "$x64zip" "$win" "$deb" "$rpm" "$clideb" "$clirpm"; do
    [ -f "$f" ] && log "present: $(basename "$f") ($(du -h "$f" | cut -f1))" || { printf 'MISSING: %s\n' "$f"; ok=0; }
  done
  # ⚠ Opens the built .app rather than trusting the packaging config. The macOS
  # auth helper is loaded by PATH at runtime, so a layout change moves it
  # somewhere the daemon does not look — and nothing fails: the daemon falls
  # back to an in-process call that cannot work under the desktop app, and
  # macOS users get a 60-second refusal with no diagnostic. This shipped once
  # already, past green unit tests and a passing developer-machine run that had
  # set BIOROUTER_AUTHPROMPT_APP and so never exercised the lookup.
  for app in "$DESK/out/Biorouter-darwin-arm64/Biorouter.app" \
             "$DESK/out/Biorouter-darwin-x64/Biorouter.app"; do
    if [ -d "$app" ]; then
      "$ROOT/scripts/check-auth-helper-bundled.sh" "$app" || ok=0
    fi
  done
  # The electron-updater manifest is generated at publish time; verify it if
  # already present (and that it references both arch zips).
  local yml="$DESK/out/make/latest-mac.yml"
  if [ -f "$yml" ]; then
    grep -q "Biorouter-darwin-arm64-$v.zip" "$yml" && grep -q "Biorouter-darwin-x64-$v.zip" "$yml" \
      && log "latest-mac.yml references both arch zips ✓" || { echo "latest-mac.yml missing an arch zip"; ok=0; }
  fi
  verify_release_provenance "$v"
  if [ -d "$DESK/out/Biorouter-darwin-arm64/Biorouter.app" ]; then
    log "arm64 gatekeeper: $(spctl --assess --type execute --verbose "$DESK/out/Biorouter-darwin-arm64/Biorouter.app" 2>&1 | tr '\n' ' ')"
    xcrun stapler validate "$DESK/out/Biorouter-darwin-arm64/Biorouter.app" >/dev/null 2>&1 && log "arm64 app stapled ✓" || { echo "arm64 NOT stapled"; ok=0; }
  fi
  if [ -d "$DESK/out/Biorouter-darwin-x64/Biorouter.app" ]; then
    file "$DESK/out/Biorouter-darwin-x64/Biorouter.app/Contents/Resources/bin/biorouterd" | grep -q x86_64 && log "intel bundled binary is x86_64 ✓" || { echo "intel binary WRONG ARCH"; ok=0; }
    xcrun stapler validate "$DESK/out/Biorouter-darwin-x64/Biorouter.app" >/dev/null 2>&1 && log "intel app stapled ✓" || { echo "intel NOT stapled"; ok=0; }
  fi
  "$ROOT/scripts/smoke-test-release-artifacts.sh" "$v" || ok=0
  [ "$ok" = 1 ] || die "verification failed"
  log "all artifacts verified"
}

# ── electron-updater macOS manifest ───────────────────────────────────────────
# latest-mac.yml is what lets the in-app "Restart & Update" button do a silent,
# one-click, in-place update on macOS (Squirrel.Mac installs from the signed
# maker-zip archives). Without it electron-updater 404s and clients fall back to
# the assisted "download to ~/Downloads" path. Re-runnable; needs both mac
# zips present (produced by `mac-arm64` + `mac-intel`).
ARM64_ZIP_REL="zip/darwin/arm64"
X64_ZIP_REL="zip/darwin/x64"
cmd_mac-manifest() {
  local v="$1"; assert_release_source "$v"; activate_hermit
  local armzip="$DESK/out/make/$ARM64_ZIP_REL/Biorouter-darwin-arm64-$v.zip"
  local x64zip="$DESK/out/make/$X64_ZIP_REL/Biorouter-darwin-x64-$v.zip"
  [ -f "$armzip" ] || die "mac arm64 zip missing — run: scripts/release.sh mac-arm64 $v"
  [ -f "$x64zip" ] || die "mac x64 zip missing — run: scripts/release.sh mac-intel $v"
  log "generating latest-mac.yml for v$v"
  # The manifest filename carries no version, so a failed or interrupted run
  # leaves the PREVIOUS release's file sitting there — and everything downstream
  # (verify, draft, and ultimately electron-updater) would accept it as this
  # release's. Remove it first so a failure is visibly missing rather than
  # quietly stale.
  rm -f "$DESK/out/make/latest-mac.yml"
  ( cd "$DESK" && node scripts/generate-update-manifests.js \
      --version "$v" --arm64-zip "$armzip" --x64-zip "$x64zip" --out "$DESK/out/make" )
  [ -f "$DESK/out/make/latest-mac.yml" ] \
    || die "manifest generation reported success but wrote no latest-mac.yml"
  record_release_asset "$v" "$DESK/out/make/latest-mac.yml"
  log "latest-mac.yml: $DESK/out/make/latest-mac.yml"
}

# ── draft + publish ───────────────────────────────────────────────────────────
release_assets() {
  local v="$1"
  printf '%s\n' \
    "$DESK/out/make/Biorouter-$v-arm64.dmg" \
    "$DESK/out/make/Biorouter-$v-x64.dmg" \
    "$DESK/out/make/$ARM64_ZIP_REL/Biorouter-darwin-arm64-$v.zip" \
    "$DESK/out/make/$X64_ZIP_REL/Biorouter-darwin-x64-$v.zip" \
    "$DESK/out/make/latest-mac.yml" \
    "$DESK/out/make/zip/win32/x64/Biorouter-win32-x64-$v.zip" \
    "$DESK/out/make/deb/x64/biorouter_${v}_amd64.deb" \
    "$DESK/out/make/rpm/x64/Biorouter-$v-1.x86_64.rpm" \
    "$ROOT/dist/cli/biorouter-cli_${v}_amd64.deb" \
    "$ROOT/dist/cli/biorouter-cli-${v}-1.x86_64.rpm"
}

require_remote_main_exact() {
  local v="$1" local_sha remote_sha remote_version
  git -C "$ROOT" fetch origin main:refs/remotes/origin/main --quiet \
    || die "could not fetch origin/main; refusing to use a potentially stale remote ref"
  local_sha="$(git -C "$ROOT" rev-parse HEAD)"
  remote_sha="$(git -C "$ROOT" rev-parse origin/main)"
  [ "$local_sha" = "$remote_sha" ] \
    || die "HEAD ($local_sha) must exactly equal origin/main ($remote_sha) before drafting or publishing"
  remote_version="$(git -C "$ROOT" show origin/main:Cargo.toml 2>/dev/null \
    | perl -0ne 'print $1 if /\[workspace\.package\].*?version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/s')"
  [ "$remote_version" = "$v" ] \
    || die "origin/main is at version '$remote_version' but this release is $v"
  RELEASE_SOURCE_SHA="$local_sha"
}

verify_remote_release_assets() {
  local v="$1" manifest releases_json stamp_file
  assert_release_source "$v"
  manifest="$(release_provenance_file "$v")"
  releases_json="$(mktemp)"
  stamp_file="$(mktemp)"
  if ! gh api "repos/$RELEASE_REPOSITORY/releases?per_page=100" >"$releases_json"; then
    rm -f "$releases_json" "$stamp_file"
    die "could not read GitHub release assets for v$v"
  fi
  if ! python3 - "$releases_json" "$manifest" "$v" "$stamp_file" <<'PY'
from datetime import datetime
import json
import os
import sys

releases_path, manifest_path, version, stamp_path = sys.argv[1:]
with open(releases_path, encoding="utf-8") as handle:
    releases = json.load(handle)
matches = [release for release in releases if release.get("tag_name") == f"v{version}"]
if len(matches) != 1:
    raise SystemExit(f"expected exactly one GitHub release tagged v{version}; found {len(matches)}")
release = matches[0]
if release.get("draft") is not True:
    raise SystemExit(f"v{version} is not a draft release")

local_assets = {}
with open(manifest_path, encoding="utf-8") as handle:
    for raw_line in handle:
        fields = raw_line.rstrip("\n").split("\t")
        if fields[0] != "asset":
            continue
        if len(fields) != 4:
            raise SystemExit("malformed release provenance asset entry")
        name = os.path.basename(fields[1])
        if name in local_assets:
            raise SystemExit(f"duplicate local release asset name: {name}")
        local_assets[name] = {"digest": fields[2].lower(), "size": int(fields[3])}

remote_assets = release.get("assets") or []
if len(local_assets) != 10 or len(remote_assets) != 10:
    raise SystemExit(
        f"expected exactly 10 local and 10 uploaded assets; found {len(local_assets)} local and {len(remote_assets)} uploaded"
    )
remote_by_name = {}
for asset in remote_assets:
    name = asset.get("name")
    if not name or name in remote_by_name:
        raise SystemExit(f"missing or duplicate uploaded asset name: {name!r}")
    remote_by_name[name] = asset
if set(remote_by_name) != set(local_assets):
    missing = sorted(set(local_assets) - set(remote_by_name))
    extra = sorted(set(remote_by_name) - set(local_assets))
    raise SystemExit(f"uploaded asset set differs from local files; missing={missing}, extra={extra}")

latest_update = None
for name, local in local_assets.items():
    remote = remote_by_name[name]
    digest = remote.get("digest")
    expected_digest = f"sha256:{local['digest']}"
    if not isinstance(digest, str) or digest.lower() != expected_digest:
        raise SystemExit(f"uploaded digest differs for {name}: expected {expected_digest}, got {digest!r}")
    if remote.get("size") != local["size"]:
        raise SystemExit(f"uploaded size differs for {name}: expected {local['size']}, got {remote.get('size')!r}")
    updated_at = remote.get("updated_at")
    if not isinstance(updated_at, str):
        raise SystemExit(f"uploaded asset has no updated_at timestamp: {name}")
    try:
        parsed = datetime.fromisoformat(updated_at.replace("Z", "+00:00"))
    except ValueError as exc:
        raise SystemExit(f"invalid uploaded asset timestamp for {name}: {updated_at}") from exc
    if latest_update is None or parsed > latest_update[0]:
        latest_update = (parsed, updated_at)
with open(stamp_path, "w", encoding="utf-8") as handle:
    handle.write(latest_update[1])
PY
  then
    rm -f "$releases_json" "$stamp_file"
    die "GitHub draft assets do not exactly match the 10 local release files"
  fi
  LATEST_DRAFT_ASSET_UPDATED_AT="$(<"$stamp_file")"
  rm -f "$releases_json" "$stamp_file"
  log "all 10 uploaded asset digests match local files"
}

require_fresh_windows_smoke() {
  local v="$1" latest_asset_update="$2" manifest runs_json source_sha
  manifest="$(release_provenance_file "$v")"
  source_sha="$(release_provenance_value "$manifest" source_sha)"
  runs_json="$(mktemp)"
  if ! gh run list --workflow release-artifact-smoke.yml --limit 100 \
    --json displayTitle,conclusion,startedAt,headSha,url >"$runs_json"; then
    rm -f "$runs_json"
    die "could not read native Windows smoke workflow runs"
  fi
  if ! python3 - "$runs_json" "$v" "$source_sha" "$latest_asset_update" <<'PY'
from datetime import datetime
import json
import sys

runs_path, version, source_sha, latest_asset_update = sys.argv[1:]
with open(runs_path, encoding="utf-8") as handle:
    runs = json.load(handle)
try:
    latest_upload = datetime.fromisoformat(latest_asset_update.replace("Z", "+00:00"))
except ValueError as exc:
    raise SystemExit(f"invalid latest draft asset timestamp: {latest_asset_update}") from exc

eligible = []
for run in runs:
    if run.get("displayTitle") != f"Release artifact smoke v{version}":
        continue
    if run.get("conclusion") != "success" or run.get("headSha") != source_sha:
        continue
    started_at = run.get("startedAt")
    if not isinstance(started_at, str):
        continue
    try:
        started = datetime.fromisoformat(started_at.replace("Z", "+00:00"))
    except ValueError:
        continue
    if started > latest_upload:
        eligible.append(run)
if not eligible:
    raise SystemExit(
        f"no successful Windows smoke for source {source_sha} started after the latest draft asset upload ({latest_asset_update})"
    )
PY
  then
    rm -f "$runs_json"
    die "native Windows release smoke must be rerun successfully after the latest draft asset upload"
  fi
  rm -f "$runs_json"
  log "native Windows smoke is newer than every draft asset upload"
}

cmd_draft() {
  local v="$1"
  local notes="$ROOT/docs/releases/notes/v$v.md"
  [ -f "$notes" ] || die "release notes missing: $notes"

  require_remote_main_exact "$v"
  cmd_mac-manifest "$v"
  verify_release_provenance "$v"
  local assets=()
  while IFS= read -r asset; do
    [ -f "$asset" ] || die "release asset missing: $asset"
    assets+=("$asset")
  done < <(release_assets "$v")
  log "creating draft GitHub release v$v"
  gh release create "v$v" --draft --target "$RELEASE_SOURCE_SHA" --title "Biorouter v$v" \
    --notes-file "$notes" "${assets[@]}"
  log "draft ready: $(gh release view "v$v" --json url --jq .url)"
}

cmd_publish() {
  local v="$1"
  require_remote_main_exact "$v"
  cmd_verify "$v"
  local is_draft
  is_draft="$(gh release view "v$v" --json isDraft --jq .isDraft 2>/dev/null || true)"
  [ "$is_draft" = true ] || die "v$v must exist as a draft release before publication"
  verify_remote_release_assets "$v"
  require_fresh_windows_smoke "$v" "$LATEST_DRAFT_ASSET_UPDATED_AT"
  gh release edit "v$v" --draft=false
  log "published: $(gh release view "v$v" --json url --jq .url)"
}

cmd_all() {
  local v="$1"
  cmd_bump "$v"; cmd_backends "$v"
  cmd_mac-arm64 "$v"; cmd_mac-intel "$v"; cmd_windows "$v"; cmd_linux "$v"
  cmd_cli-linux "$v"                                                    # CLI-only deb/rpm (no GUI)
  ( cd "$DESK" && npm ci >/dev/null 2>&1 )
  # Before verify, not after: verify inspects latest-mac.yml, so generating it
  # only inside cmd_draft left a full run checking a manifest from the PREVIOUS
  # release. cmd_mac-manifest is idempotent, so cmd_draft's own call can stay and
  # `draft` remains correct when run standalone.
  cmd_mac-manifest "$v"
  cmd_verify "$v"; cmd_draft "$v"
  log "draft created; run the native Windows smoke workflow, then: scripts/release.sh publish $v"
}

# ── version resolution ───────────────────────────────────────────────────────
# Accepts a literal `X.Y.Z` (optionally `vX.Y.Z`, since that is how the tags are
# spelled and it is the obvious thing to paste), or one of three keywords that
# compute the next version from the tree's current one:
#
#   major        1.88.6 → 2.0.0     the FIRST number; resets the other two
#   minor        1.88.6 → 1.89.0    the SECOND number; resets the third
#   patch        1.88.6 → 1.88.7    the THIRD number
#
# `minor-minor` is accepted as an alias for `patch`, because "the minor minor
# version" is how the third number gets described in practice and a script that
# rejects the user's own vocabulary is a script people stop using.
current_version() {
  perl -0ne 'print $1 if /\[workspace\.package\].*?version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/s' Cargo.toml
}

resolve_version() {
  local want="$1" cur part
  cur="$(current_version)"
  [ -n "$cur" ] || die "could not read the current version from Cargo.toml [workspace.package]"

  case "$want" in
    major|minor|patch|minor-minor)
      part="$want"
      [ "$part" = minor-minor ] && part=patch
      IFS=. read -r a b c <<<"$cur"
      case "$part" in
        major) echo "$((a + 1)).0.0" ;;
        minor) echo "$a.$((b + 1)).0" ;;
        patch) echo "$a.$b.$((c + 1))" ;;
      esac
      ;;
    v[0-9]*.[0-9]*.[0-9]*|[0-9]*.[0-9]*.[0-9]*)
      local v="${want#v}"
      [[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] \
        || die "malformed version '$want' — expected X.Y.Z, or one of: major minor patch"
      # Going backwards silently is a real hazard: electron-updater compares
      # versions, so a lower number ships an update clients will refuse and a
      # `latest-mac.yml` that disagrees with its own assets.
      if [ "$(printf '%s\n%s\n' "$cur" "$v" | sort -V | tail -1)" = "$cur" ] && [ "$v" != "$cur" ]; then
        die "refusing to bump BACKWARDS: $cur → $v. Pass the version explicitly again if this is deliberate — but check the update feed first."
      fi
      echo "$v"
      ;;
    *)
      die "unrecognised version '$want' — expected X.Y.Z, or one of: major minor patch (minor-minor = patch)"
      ;;
  esac
}

# Focused tests source this file to exercise the fail-closed helpers with local
# fixtures. Direct invocations continue through the command dispatcher below.
if [ "${BASH_SOURCE[0]}" != "$0" ]; then
  return 0
fi

CMD="${1:-}"; VER="${2:-}"
case "$CMD" in
  bump|all)
    need_version "$VER"
    RESOLVED="$(resolve_version "$VER")"
    if [ "$RESOLVED" != "$VER" ]; then
      log "resolved '$VER' → $RESOLVED (current: $(current_version))"
    fi
    "cmd_${CMD}" "$RESOLVED"
    # ⚠ `if`, not `[ … ] && log …`. As an `&&` chain this was the case arm's LAST
    # statement, so on `all` the false test made the whole script exit 1 after a
    # completely successful release — and a caller checking the status would have
    # treated a good run as a failed one.
    if [ "$CMD" = bump ]; then
      log "later phases take this explicitly, e.g. scripts/release.sh backends $RESOLVED"
    fi
    ;;
  backends|linux-backend|mac-arm64|mac-intel|mac-manifest|windows|linux|cli-linux|verify|draft|publish)
    need_version "$VER"
    # Keywords are deliberately REFUSED here. These phases run against a tree
    # that `bump` has already rewritten, so `minor` would resolve against the
    # NEW current version and compute a version one step too far — building
    # 1.90.0 artifacts for a 1.89.0 tree, with nothing to catch it until verify.
    case "$VER" in
      major|minor|patch|minor-minor)
        die "'$VER' is only valid for 'bump' and 'all'. This phase needs the explicit version the tree is already at: $(current_version)" ;;
    esac
    "cmd_${CMD}" "$VER" ;;
  *) die "usage: scripts/release.sh {bump|backends|linux-backend|mac-arm64|mac-intel|mac-manifest|windows|linux|cli-linux|verify|draft|publish|all} <version|major|minor|patch>" ;;
esac
