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
#   adopt-ci <ver>    Take the 4 Linux + 2 Windows assets from the successful
#                     linux-gui-packages.yml / windows-gui-packages.yml runs
#                     at this release's source commit.
#   windows <ver>     Package the Windows .zip + installer locally (retired
#                     from the release path; adopt-ci replaces it).
#   linux <ver>       Package the GUI .deb + .rpm locally (retired; adopt-ci).
#   cli-linux <ver>   Build the CLI-only .deb + .rpm locally (retired; adopt-ci).
#   mac-manifest <ver>
#                     Generate latest-mac.yml for electron-updater.
#   verify <ver>      Verify all release artifacts (arch, notarization, dmg format).
#   draft <ver>       Create a draft GitHub release with assets + notes.
#   publish <ver>     Publish a verified draft after native Windows smoke passes.
#   landing <ver>     Point the landing site at a PUBLISHED release.
#   all <ver>         Run every build/verify phase, adopt the CI packages, and
#                     create the draft release.
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
      # A manifest, not an archive.
      *.yml) ;;
      # The Squirrel installer. `verify-computer-use-artifact.py` unpacks
      # .zip/.dmg/.deb/.rpm and refuses a PE, so this one cannot be attested
      # directly; it is built from the same staged app directory as the
      # attested win32 zip. Do not "fix" this by dropping the attestation for
      # everything -- narrow the exemption, not the rule.
      *.exe) ;;
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
  # 11 since the Windows Squirrel installer joined the set. This count is a
  # tripwire for an asset list edited in one place and not the others -- keep
  # it in step with `release_assets`, `cmd_verify` and CLAUDE.md.
  [ "$expected_count" -eq 11 ] || die "internal release asset list changed; expected exactly 11 assets"
  local helper_count
  helper_count="$(awk -F '\t' '$1 == "computer_use" { count++ } END { print count+0 }' "$manifest")"
  # 9, not 11: two of the eleven carry no helper attestation of their own.
  # `latest-mac.yml` is a manifest rather than an archive, and
  # `Biorouter-Setup-<ver>.exe` is a Squirrel installer --
  # `verify-computer-use-artifact.py` reads .zip/.dmg/.deb/.rpm and dies with
  # "Unsupported release archive" on a PE. The installer is built from the same
  # staged app directory as `Biorouter-win32-x64-<ver>.zip`, which IS attested,
  # so the win32 helper bytes are covered -- but covered indirectly. Teaching
  # the verifier to open a Squirrel exe (its embedded nupkg) would make this
  # direct and is worth doing separately.
  [ "$helper_count" -eq 9 ] || die "release provenance must attest the helper bytes in all 9 attestable install/update archives"
  verify_release_ci_runs "$v"
  log "release provenance verified for 11 assets at $(release_provenance_value "$manifest" source_sha)"
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

# Linux x86_64 backend (biorouterd + biorouter + biorouter-crew). Extracted so it can be re-run
# on its own. Cleans the target dir first to force a from-scratch compile
# against the pinned glibc (cached objects would keep stale symbol versions).
cmd_linux-backend() {
  local v="$1"
  assert_release_source "$v"
  ensure_docker
  log "cross-compiling linux-gnu backend (docker, $LINUX_RUST_IMG)"
  rm -rf "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter" \
         "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouterd" \
         "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter-crew"
  run_cross_release \
    cross_linux \
    biorouter-linux-release-target \
    "cargo build --release --bin biorouterd --bin biorouter --bin biorouter-crew" \
    "mkdir -p /usr/src/myapp/target/x86_64-unknown-linux-gnu/release && \
     cp -f /cross-target/x86_64-unknown-linux-gnu/release/biorouter \
           /cross-target/x86_64-unknown-linux-gnu/release/biorouterd \
           /cross-target/x86_64-unknown-linux-gnu/release/biorouter-crew \
           /usr/src/myapp/target/x86_64-unknown-linux-gnu/release/"
  assert_glibc_floor
  # The broker must carry `join-by-name` (on by default since 2026-09-25, naming design D17).
  # Nothing else notices a broker built without it: it starts, answers --version and --help,
  # and only refuses to let anyone join by invitation.
  bash "$ROOT/scripts/check-crew-broker-join.sh" \
    "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter-crew" \
    || die "the linux Crew broker was built without the join-by-name feature"
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
             "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter" \
             "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter-crew"; do
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
  # Only the payloads the release path packages here. The win32 and linux ones
  # are built by the phases that package them (see build_computer_use_helper).
  for helper_target in darwin-arm64 darwin-x64; do
    build_computer_use_helper "$helper_target"
  done
  assert_release_source "$v"
  log "all 4 backends compiled"
}

# Build one Biorouter Copilot helper payload into target/computer-use/<target>.
# The darwin helpers are Swift, built with the Xcode toolchain the mac phases
# already need. The win32 and linux helpers are Go, and only the local windows,
# linux and cli-linux phases consume them; the release path takes those
# packages from CI (adopt-ci), which installs Go itself. So `backends` builds
# the two darwin payloads and each of those phases builds its own. Measured
# 2026-09-22: v1.91.1's `backends` died on `go` not found, after every Rust
# backend had compiled, building payloads nothing on the release path reads.
build_computer_use_helper() { # <target>
  local target="$1"
  case "$target" in
    darwin-*) ;;
    *) command -v go >/dev/null 2>&1 \
         || die "the $target Biorouter Copilot helper is built with Go, which is not on PATH. Install Go (.github/workflows/computer-use-native.yml pins the version CI uses), or take this platform's packages from CI: scripts/release.sh adopt-ci <version>" ;;
  esac
  python3 "$ROOT/scripts/computer-use-runtime.py" build "$target" --signing-identity "$SIGN_IDENTITY"
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
# Not on the release path: adopt-ci takes both Windows assets from one windows-gui-packages.yml run, because a zip from here and an installer from Windows carried different backends (see the adopt-ci block).
cmd_windows() {
  local v="$1"; assert_release_source "$v"; activate_hermit; ensure_host_node_deps
  local WR="$ROOT/target/x86_64-pc-windows-gnu/release"
  [ -f "$WR/biorouterd.exe" ] || die "windows backend missing — run: scripts/release.sh backends $v"
  build_computer_use_helper win32-x64
  rm -rf "$DESK/src/bin"; mkdir -p "$DESK/src/bin"
  cp -f "$WR/biorouterd.exe" "$WR/biorouter.exe" "$WR"/*.dll "$DESK/src/bin/"
  log "packaging Windows zip"
  ( cd "$DESK" && npm run bundle:windows )
  local zip="$DESK/out/make/zip/win32/x64/Biorouter-win32-x64-$v.zip"
  [ -f "$zip" ] || die "windows reported success but produced no zip at $zip"
  record_release_asset "$v" "$zip" "win32-x64"
  log "windows zip: $zip"
  # The Squirrel installer is what makes a Windows update in-place: running it
  # over an existing install replaces the app directory and keeps the shortcuts,
  # where the zip leaves the user to extract and swap a folder by hand. The
  # updater looks for this exact filename (forge.config.ts WINDOWS_SETUP_EXE,
  # githubUpdater.ts), so a missing or misnamed one silently sends Windows back
  # to the assisted download.
  local setup="$DESK/out/make/squirrel.windows/x64/Biorouter-Setup-$v.exe"
  [ -f "$setup" ] || die "windows produced no installer at $setup - is maker-squirrel still in forge.config.ts?"
  record_release_asset "$v" "$setup"
  log "windows installer: $setup"
}

# ── linux packaging (fully dockerized; run LAST — corrupts node_modules) ───────
# Not on the release path: adopt-ci takes the GUI deb/rpm from linux-gui-packages.yml, because Docker Desktop on this host left the container's computer-use files unreadable (see the adopt-ci block).
cmd_linux() {
  local v="$1"; assert_release_source "$v"; ensure_docker
  [ -f "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouterd" ] || die "linux backend missing — run: scripts/release.sh backends $v"
  build_computer_use_helper linux-x64
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
# Not on the release path: adopt-ci takes the CLI deb/rpm from the same linux-gui-packages.yml run as the GUI ones, so all four carry one backend build rather than this host's target/ beside CI's.
cmd_cli-linux() {
  # ensure_host_node_deps because these packages now carry the browser interface
  # bundle, which is built on the HOST by `npm run build:web`. After a Linux or
  # Windows docker build the on-disk node_modules is Linux-flavored and that
  # build dies on a missing @rollup/rollup-darwin-arm64 — a failure that reads
  # as a rollup bug rather than an ordering one. The retired headless phase had
  # this call for the same reason; it is needed here now that the payload moved.
  local v="$1"; assert_release_source "$v"; ensure_docker; ensure_host_node_deps
  [ -f "$ROOT/target/x86_64-unknown-linux-gnu/release/biorouter" ] || die "linux backend missing — run: scripts/release.sh backends $v"
  build_computer_use_helper linux-x64
  log "building CLI-only Linux packages (deb + rpm)"
  bash "$ROOT/scripts/build-cli-linux-packages.sh" "$v"
  record_release_asset "$v" "$ROOT/dist/cli/biorouter-cli_${v}_amd64.deb" "linux-x64"
  record_release_asset "$v" "$ROOT/dist/cli/biorouter-cli-${v}-1.x86_64.rpm" "linux-x64"
  log "cli deb: $ROOT/dist/cli/biorouter-cli_${v}_amd64.deb"
  log "cli rpm: $ROOT/dist/cli/biorouter-cli-${v}-1.x86_64.rpm"
}

# ── adopt artifacts built by CI ───────────────────────────────────────────────
# Six of the eleven release assets are built by GitHub Actions rather than on
# this host: the four Linux packages (`linux-gui-packages.yml`) and the two
# Windows ones (`windows-gui-packages.yml`). Both moves were forced by a
# measured defect, not by preference.
#
#   Linux  — Docker Desktop on macOS reports permissions from its
#            `com.docker.grpcfuse.ownership` xattr rather than the inode, so
#            every file the container wrote into `out/resources/computer-use/`
#            came back `--w-------` and the packager could not read what it had
#            just written. Three attempts, source verified clean each time; a
#            named volume instead of the bind mount changed nothing.
#   Windows — building the zip here and the installer on a Windows machine gave
#            one release two Windows artifacts with DIFFERENT backends. Same
#            commit, both correct, inner biorouter.exe 4d8ce07c… versus
#            7961b022…, because Rust builds embed paths and are not reproducible
#            by default. Nothing recorded it: the provenance manifest hashes
#            each archive and never the binaries inside it.
#
# ⚠ THE WHOLE POINT OF THIS PHASE IS WHERE THE PROVENANCE COMES FROM.
# `assert_release_source` checks HEAD and tree cleanliness. That says exactly
# nothing about a file someone downloaded and dropped into `out/make` — the
# same blind spot that let a 1.91.0 installer ship a 1.90.5 backend with the
# stamp matching HEAD throughout, because the stamp described `target/` while
# the packager read `src/bin`. A provenance check on the SOURCE of a copy says
# nothing about a build that READ SOMEWHERE ELSE.
#
# So the run is SELECTED BY head sha rather than checked afterwards: only a
# successful run of the named workflow whose `headSha` equals this release's
# `source_sha` is eligible at all, and the chosen run id is written into the
# manifest so the claim stays checkable after the fact. An eligible run that
# does not exist is a hard failure, never a fallback to whatever is newest.
RELEASE_CI_WORKFLOWS="linux-gui-packages.yml windows-gui-packages.yml"

# Exit status: 0 with the run id on stdout; 1 when GitHub answered and no run
# qualifies; 2 when GitHub could not be asked at all. The two failures need
# opposite fixes (dispatch a run, versus repair gh), and this used to send gh's
# stderr to /dev/null and return 1 for both, so an auth, network or rate-limit
# failure was reported as "no successful run -- dispatch it and wait".
ci_run_for_release() { # <workflow file> <source sha> -> run id on stdout
  local wf="$1" sha="$2" id
  id="$(gh run list --workflow "$wf" --limit 50 \
          --json databaseId,headSha,status,conclusion \
          --jq "[.[] | select(.headSha == \"$sha\" and .status == \"completed\" and .conclusion == \"success\")] | first | .databaseId")" \
    || return 2
  [ -n "$id" ] && [ "$id" != "null" ] || return 1
  printf '%s\n' "$id"
}

# The refusal for each of ci_run_for_release's failures, naming the fix that
# actually applies to it.
ci_run_refusal() { # <workflow file> <source sha> <version> <status>
  case "$4" in
    0) ;;
    1)
      # ⚠ Status 1 means "no COMPLETED, SUCCESSFUL run at this sha", which is three
      # different situations, and only one of them wants a dispatch. Telling
      # someone to dispatch while a run is already queued or in progress creates a
      # DUPLICATE run for the same commit, and a failed run needs its cause fixed
      # first — re-dispatching an unchanged commit just fails again. So say which.
      local latest st con rid
      latest="$(gh run list --workflow "$1" --limit 50 --json databaseId,headSha,status,conclusion \
                  --jq "[.[] | select(.headSha == \"$2\")] | first | \"\\(.databaseId) \\(.status) \\(.conclusion)\"" 2>/dev/null || true)"
      read -r rid st con <<<"$latest"
      if [ -z "$rid" ] || [ "$rid" = "null" ]; then
        die "no $1 run at $2. Dispatch it and wait for it to succeed: gh workflow run $1 -f version=$3 --ref main (it builds main's head, so $2 must be origin/main when you dispatch)"
      elif [ "$st" != "completed" ]; then
        die "$1 run $rid at $2 is still $st. Wait for it — do NOT dispatch another, that would build the same commit twice: gh run watch $rid"
      else
        die "$1 run $rid at $2 completed with conclusion '$con', not success. Fix the cause before dispatching again; re-running an unchanged commit repeats the failure: gh run view $rid --log-failed"
      fi ;;
    *) die "could not query GitHub for $1 runs (gh's own error is above). That is not a missing run, so do not dispatch one; check 'gh auth status' and the network, then re-run" ;;
  esac
}

record_ci_run() { # <version> <workflow> <run id>
  local v="$1" wf="$2" id="$3" manifest tmp
  manifest="$(release_provenance_file "$v")"
  tmp="$(mktemp "${manifest}.XXXXXX")"
  awk -F '\t' -v wf="$wf" '!($1 == "ci_run" && $2 == wf)' "$manifest" >"$tmp"
  printf 'ci_run\t%s\t%s\n' "$wf" "$id" >>"$tmp"
  mv "$tmp" "$manifest"
}

# Reads the `ci_run` rows back. adopt-ci writes them, and until this check
# nothing ever read them, so the one record of where six assets came from was
# never compared with anything. A manifest with no such rows is a release
# packaged entirely on this host, which is left to the checks above. Otherwise
# every CI workflow must be named exactly once, and GitHub must still report
# that run as a successful run OF THAT WORKFLOW at THIS release's source
# commit. The REST run object rather than `gh run view`, because only it
# carries the workflow file (`path`); without it a row naming the other
# workflow's run passes whenever both built the same commit, which is exactly
# the case adopt-ci selects for. Same repository resolution as
# ci_run_for_release, so the id is read where it was chosen.
verify_release_ci_runs() { # <version>
  local v="$1" manifest source_sha bad wf count id fields head status conclusion path
  manifest="$(release_provenance_file "$v")"
  awk -F '\t' '$1 == "ci_run" { found = 1 } END { exit !found }' "$manifest" || return 0
  source_sha="$(release_provenance_value "$manifest" source_sha)"
  bad="$(awk -F '\t' -v wfs="$RELEASE_CI_WORKFLOWS" '
    BEGIN { n = split(wfs, w, " "); for (i = 1; i <= n; i++) known[w[i]] = 1 }
    $1 == "ci_run" && (NF != 3 || !($2 in known) || $3 !~ /^[0-9]+$/)
  ' "$manifest")"
  [ -z "$bad" ] || die "malformed ci_run row in release provenance: $bad"
  for wf in $RELEASE_CI_WORKFLOWS; do
    count="$(awk -F '\t' -v wf="$wf" '$1 == "ci_run" && $2 == wf { c++ } END { print c+0 }' "$manifest")"
    [ "$count" -eq 1 ] \
      || die "release provenance names $count $wf runs; adopt-ci records exactly one per workflow. Re-run: scripts/release.sh adopt-ci $v"
    id="$(awk -F '\t' -v wf="$wf" '$1 == "ci_run" && $2 == wf { print $3 }' "$manifest")"
    # `conclusion` is null until a run completes, and `read` collapses a run of
    # tabs, so the one field that can be empty goes last.
    fields="$(gh api "repos/{owner}/{repo}/actions/runs/$id" --jq '[.path, .head_sha, .status, .conclusion] | @tsv')" \
      || die "could not read $wf run $id from GitHub (gh's own error is above), so the CI-built assets cannot be checked"
    IFS=$'\t' read -r path head status conclusion <<<"$fields"
    [ "$path" = ".github/workflows/$wf" ] \
      || die "release provenance names run $id for $wf, but that run is of ${path:-an unknown workflow}. Re-run: scripts/release.sh adopt-ci $v"
    [ "$head" = "$source_sha" ] \
      || die "$wf run $id built $head, not this release's source $source_sha. Run the workflow at $source_sha, then: scripts/release.sh adopt-ci $v"
    [ "$status" = completed ] && [ "$conclusion" = success ] \
      || die "$wf run $id is $status/$conclusion, not completed/success. Re-run: scripts/release.sh adopt-ci $v"
    log "$wf run $id: success at $source_sha"
  done
}

# ⚠ A SUBSHELL body, `( ... )` rather than `{ ... }`, so the staging directory
# is removed on every way out. It holds both downloaded artifacts, and the
# Windows one alone is 809,507,441 bytes (windows-packages-1.91.0, run
# 35554562885). The cleanup used to be `trap ... RETURN`, which never fires on
# `die` or on an errexit, because both exit the shell rather than return, so
# every failed adoption leaked it. A RETURN trap also stays set after the
# function returns: called from another function, as cmd_all now does, it
# fires again when that caller returns, dies on `stage: unbound variable`
# under `set -u`, and turns a fully successful run into exit 1. An EXIT trap
# set inside the subshell fires when the subshell ends, however it ends, and
# neither the trap nor `adopt_one` reaches the caller. Measured under /bin/bash
# 3.2.57 and bash 5.3.20, the old shape and this one side by side: the old
# leaked the directory on `die` and exited 1 on a nested success; this one
# removed it on success, die, errexit and a die inside a nested function, kept
# exit status 1 on failure and 0 on success, and left the caller no EXIT trap
# and no `adopt_one`. Nothing the caller reads is set here; the results are
# the manifest rows and the copied files.
cmd_adopt-ci() (
  local v="$1"
  assert_release_source "$v"
  command -v gh >/dev/null 2>&1 || die "adopt-ci needs the gh CLI"
  local manifest sha
  manifest="$(release_provenance_file "$v")"
  sha="$(release_provenance_value "$manifest" source_sha)"

  # Resolve BOTH runs before downloading anything. Half-adopting a release is
  # worse than not starting: `verify` would then report a specific missing file
  # and read as a build problem rather than as a missing CI run.
  local lin win rc
  rc=0; lin="$(ci_run_for_release linux-gui-packages.yml "$sha")" || rc=$?
  ci_run_refusal linux-gui-packages.yml "$sha" "$v" "$rc"
  rc=0; win="$(ci_run_for_release windows-gui-packages.yml "$sha")" || rc=$?
  ci_run_refusal windows-gui-packages.yml "$sha" "$v" "$rc"
  log "adopting linux-gui-packages.yml run $lin and windows-gui-packages.yml run $win (both at $sha)"

  local stage; stage="$(mktemp -d)"
  trap 'rm -rf -- "$stage"' EXIT
  gh run download "$lin" -n "linux-packages-$v" -D "$stage/linux" \
    || die "could not download linux-packages-$v from run $lin"
  gh run download "$win" -n "windows-packages-$v" -D "$stage/windows" \
    || die "could not download windows-packages-$v from run $win"

  # `gh run download` reproduces the paths the workflow uploaded, so find the
  # files by name rather than assuming a layout that an upload-path edit would
  # silently change.
  adopt_one() { # <staged root> <basename> <destination dir> [runtime target]
    local root="$1" name="$2" dest="$3" target="${4:-}" src
    src="$(find "$root" -type f -name "$name" -print -quit)"
    [ -n "$src" ] || die "run artifact did not contain $name"
    mkdir -p "$dest"
    cp -f "$src" "$dest/$name"
    record_release_asset "$v" "$dest/$name" "$target"
  }

  adopt_one "$stage/linux" "biorouter_${v}_amd64.deb"        "$DESK/out/make/deb/x64" linux-x64
  adopt_one "$stage/linux" "Biorouter-$v-1.x86_64.rpm"       "$DESK/out/make/rpm/x64" linux-x64
  adopt_one "$stage/linux" "biorouter-cli_${v}_amd64.deb"    "$ROOT/dist/cli"         linux-x64
  adopt_one "$stage/linux" "biorouter-cli-${v}-1.x86_64.rpm" "$ROOT/dist/cli"         linux-x64
  adopt_one "$stage/windows" "Biorouter-win32-x64-$v.zip"    "$DESK/out/make/zip/win32/x64" win32-x64
  # No runtime target: verify-computer-use-artifact.py reads .zip/.dmg/.deb/.rpm
  # and dies on a PE. Covered indirectly by the attested win32 zip, which this
  # workflow builds from the same staged tree in the same job.
  adopt_one "$stage/windows" "Biorouter-Setup-$v.exe"        "$DESK/out/make/squirrel.windows/x64"

  record_ci_run "$v" linux-gui-packages.yml "$lin"
  record_ci_run "$v" windows-gui-packages.yml "$win"
  log "adopted 6 CI-built assets; provenance records runs $lin and $win"
)

# ── verify ────────────────────────────────────────────────────────────────────
cmd_verify() {
  local v="$1" ok=1
  assert_release_source "$v"
  # Every non-mac smoke in smoke-test-release-artifacts.sh is a `docker run`.
  # Verify used to rely on cmd_linux / cmd_cli-linux having started Docker just
  # before it; neither runs on the release path now that adopt-ci replaced
  # them, and publish re-runs this. Started here, a missing daemon fails in
  # seconds instead of after the mac checks.
  ensure_docker
  "$ROOT/scripts/check-brand-consistency.sh"
  local arm="$DESK/out/make/Biorouter-$v-arm64.dmg"
  local x64="$DESK/out/make/Biorouter-$v-x64.dmg"
  local win="$DESK/out/make/zip/win32/x64/Biorouter-win32-x64-$v.zip"
  local winsetup="$DESK/out/make/squirrel.windows/x64/Biorouter-Setup-$v.exe"
  local deb="$DESK/out/make/deb/x64/biorouter_${v}_amd64.deb"
  local rpm="$DESK/out/make/rpm/x64/Biorouter-$v-1.x86_64.rpm"
  local clideb="$ROOT/dist/cli/biorouter-cli_${v}_amd64.deb"
  local clirpm="$ROOT/dist/cli/biorouter-cli-${v}-1.x86_64.rpm"
  local armzip="$DESK/out/make/$ARM64_ZIP_REL/Biorouter-darwin-arm64-$v.zip"
  local x64zip="$DESK/out/make/$X64_ZIP_REL/Biorouter-darwin-x64-$v.zip"
  for f in "$arm" "$x64" "$armzip" "$x64zip" "$win" "$winsetup" "$deb" "$rpm" "$clideb" "$clirpm"; do
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
    "$DESK/out/make/squirrel.windows/x64/Biorouter-Setup-$v.exe" \
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
if len(local_assets) != 11 or len(remote_assets) != 11:
    raise SystemExit(
        f"expected exactly 11 local and 11 uploaded assets; found {len(local_assets)} local and {len(remote_assets)} uploaded"
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
    die "GitHub draft assets do not exactly match the 11 local release files"
  fi
  LATEST_DRAFT_ASSET_UPDATED_AT="$(<"$stamp_file")"
  rm -f "$releases_json" "$stamp_file"
  log "all 11 uploaded asset digests match local files"
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
  # ⚠ CHECK the tag's target; do not derive it. publish creates the tag from the
  # draft's recorded targetCommitish, not from today's main. cmd_draft pins that to
  # RELEASE_SOURCE_SHA, and require_remote_main_exact (above) sets that variable
  # from HEAD only after asserting HEAD == a freshly fetched origin/main, so for a
  # draft THIS script made the chain holds by construction. But nothing ever read
  # the target back, so a draft made any other way — a hand-run `gh release
  # create`, or a release.sh from before the pin, when drafts took `--target main`
  # — would publish a tag at a commit the artifacts were never built from, and
  # every check here would pass. That is v1.89.8: tag and artifacts 8 commits
  # apart. A guarantee derived from an invariant is not a guarantee checked.
  # ⚠ `local` on its own line, deliberately. `local x="$(cmd)"` returns the exit
  # status of `local` itself — effectively always 0 — so the `|| die` below would
  # never fire and an unreadable target would continue as an empty string. A
  # plain assignment carries the command substitution's own status. Do not fold
  # these two lines back into one.
  local draft_target
  draft_target="$(gh release view "v$v" --json targetCommitish --jq .targetCommitish)" \
    || die "could not read v$v's draft target from GitHub; refusing to publish without it"
  [ "$draft_target" = "$RELEASE_SOURCE_SHA" ] \
    || die "v$v's draft targets $draft_target, but the artifacts were built at $RELEASE_SOURCE_SHA. Publishing would tag a commit the release was not built from. Delete the draft and re-run: scripts/release.sh draft $v"
  log "draft target is the source commit ($draft_target)"
  verify_remote_release_assets "$v"
  require_fresh_windows_smoke "$v" "$LATEST_DRAFT_ASSET_UPDATED_AT"
  gh release edit "v$v" --draft=false
  log "published: $(gh release view "v$v" --json url --jq .url)"
  log "next: scripts/release.sh landing $v   (the public site still cites the previous release)"
}

# ── landing site ──────────────────────────────────────────────────────────────
# The public site cites the LATEST PUBLISHED version, not the tree's. Nothing in
# this script used to touch `landing/` at all, which is why the site sat three
# releases behind: its download links, install commands and version badges all
# named v1.88.x while the tree was on 1.90.5. Its own guard
# (`landing/scripts/check-consistency.mjs`) had been failing that whole time.
#
# ⚠ This runs AFTER publish, deliberately, and refuses to run before. The site's
# hardcoded versions are FALLBACKS used when GitHub is unreachable, so pointing
# them at an unpublished version would make every download link 404 in exactly
# the case the fallback exists to cover.
cmd_landing() {
  local v="$1"
  local is_draft
  is_draft="$(gh release view "v$v" --json isDraft --jq .isDraft 2>/dev/null || echo missing)"
  case "$is_draft" in
    false) ;;
    true) die "v$v is still a draft. The landing site's versions are fallbacks for when GitHub is unreachable, so they must name a published release. Run: scripts/release.sh publish $v" ;;
    *) die "v$v is not a published release; refusing to point the landing site at it" ;;
  esac

  log "pointing the landing site at the published v$v"
  local content=landing/assets/landing-site-content.md about=landing/about.html

  # ⚠ The News lists are HISTORY, and prose. about.html names a release nowhere
  # but its `.news-list`, and the `### News` section is the one part of
  # content.md that names past releases; every row in either is an older
  # release's headline and summary.
  # A mechanical rewrite cannot write the new row and would relabel the newest
  # old one. Run on a copy of the 1.90.5 site, this phase's original loop (one
  # global replace per file) turned content.md's "Biorouter v1.90.5 Release"
  # entry into v1.91.0 while keeping 1.90.5's summary, and the about.html it
  # never touched then failed check-consistency.mjs's "about news should link
  # to the latest published release", so the first real run could not pass. So
  # the new rows are a person's edit, and this checks for them BEFORE touching
  # any file, so a refusal leaves the tree exactly as it was.
  local about_tag news_tag missing=""
  about_tag="$(perl -0ne 'my ($list) = /class="news-list">(.*)/s or exit; print $1 if $list =~ m{releases/tag/v([0-9]+\.[0-9]+\.[0-9]+)}' "$about")"
  news_tag="$(perl -0ne 'my ($news) = /^### News\n(.*?)(?=^#{1,3} |\z)/ms or exit; print $1 if $news =~ m{releases/tag/v([0-9]+\.[0-9]+\.[0-9]+)}' "$content")"
  [ "$about_tag" = "$v" ] \
    || missing="$missing
  - $about: its newest news row links v${about_tag:-<none found>}. Add a row above it in the .news-list, in that row's shape: href https://github.com/BaranziniLab/biorouter/releases/tag/v$v, the publish day and month, an <h3> headline and a <p> summary from docs/releases/notes/v$v.md."
  [ "$news_tag" = "$v" ] \
    || missing="$missing
  - $content: its newest '### News' entry links v${news_tag:-<none found>}. Add '1. **Biorouter v$v Release**' with its Link and What's new above it, and renumber the entries below."
  [ -z "$missing" ] || die "the landing News lists have no entry for v$v, and this phase does not write prose:$missing
Then re-run: scripts/release.sh landing $v"

  local prev
  prev="$(perl -ne 'print $1 and exit if /\*\*Version:\*\* v([0-9]+\.[0-9]+\.[0-9]+)/' "$content")"
  [ -n "$prev" ] || die "could not read the current landing version from $content"
  if [ "$prev" = "$v" ]; then
    log "landing site already cites v$v"
  else
    log "landing site: v$prev → v$v"
    local f
    for f in "$content" landing/index.html landing/download.html landing/docs.html; do
      [ -f "$f" ] || continue
      if [ "$f" = "$content" ]; then
        # Every current-release slot EXCEPT the News section, which keeps the
        # versions its rows were written about. The check above already
        # required the section; the `die` is for a file that changed between.
        PREV="$prev" NEW="$v" perl -0pi -e '
          /^### News\n.*?(?=^#{1,3} |\z)/ms or die "no ### News section in $ARGV\n";
          my ($s, $e) = ($-[0], $+[0]);
          my ($head, $news, $tail) = (substr($_, 0, $s), substr($_, $s, $e - $s), substr($_, $e));
          s/\Q$ENV{PREV}\E/$ENV{NEW}/g for $head, $tail;
          $_ = $head . $news . $tail;
        ' "$f"
      else
        perl -0pi -e "s/\Q$prev\E/$v/g" "$f"
      fi
    done
  fi

  # The site's own guard is the check, not this function's diff.
  ( cd landing && node scripts/check-consistency.mjs ) \
    || die "landing consistency checks failed after the version update; fix the site before committing"
  log "landing site updated and its consistency checks pass"
  log "⚠ this leaves an uncommitted change; commit landing/ and push so the site deploys"
}

# Reports, without failing, whether the two CI packaging runs adopt-ci needs
# already exist. They are the one step `all` cannot perform itself, and they
# can build while the mac phases notarize, so the time to say so is before the
# notarization rather than after it. adopt-ci makes the binding check.
ci_packaging_hint() { # <version>
  local v="$1" sha wf rc
  sha="$(release_provenance_value "$(release_provenance_file "$v")" source_sha)"
  for wf in $RELEASE_CI_WORKFLOWS; do
    rc=0; ci_run_for_release "$wf" "$sha" >/dev/null || rc=$?
    case "$rc" in
      0) log "$wf already has a successful run at $sha" ;;
      1) log "⚠ $wf has no successful run at $sha yet. Dispatch it now, so it builds during notarization: gh workflow run $wf -f version=$v --ref main (it builds main's head, so $sha must be origin/main when you dispatch)" ;;
      *) log "⚠ could not ask GitHub about $wf runs (gh's own error is above); adopt-ci will ask again after the mac phases" ;;
    esac
  done
}

cmd_all() {
  local v="$1"
  cmd_bump "$v"; cmd_backends "$v"
  ci_packaging_hint "$v"
  cmd_mac-arm64 "$v"; cmd_mac-intel "$v"
  # The four Linux and two Windows assets come from CI, not from the local
  # windows / linux / cli-linux phases (see the adopt-ci block for the measured
  # reasons). adopt-ci dies naming the exact dispatch command when a run is
  # missing, and the phases after it resume one at a time. The `npm ci` that
  # used to sit here repaired node_modules after the local Linux docker
  # package, which no longer runs.
  cmd_adopt-ci "$v"
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
  backends|linux-backend|mac-arm64|mac-intel|mac-manifest|windows|linux|cli-linux|adopt-ci|verify|draft|publish|landing)
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
  *) die "usage: scripts/release.sh {bump|backends|linux-backend|mac-arm64|mac-intel|mac-manifest|windows|linux|cli-linux|adopt-ci|verify|draft|publish|landing|all} <version|major|minor|patch>" ;;
esac
