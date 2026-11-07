#!/usr/bin/env bash
# Execute the shipped vX.Y.Z artifacts in their target environments.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DESK="$ROOT/ui/desktop"
VERSION="${1:?usage: scripts/smoke-test-release-artifacts.sh <version> [all|mac|deb|rpm|cli|serve]}"
TARGET="${2:-all}"

log() { printf '\033[1;36m[release-smoke]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[release-smoke] %s\033[0m\n' "$*" >&2; exit 1; }

require_file() {
  [ -f "$1" ] || die "missing artifact: $1"
}

# The microphone (dictation, DD-65), read back out of the signed app: the four
# files `macSignOptionsForFile` in ui/desktop/forge.config.ts signs with
# `com.apple.security.device.audio-input`, and its `NSMicrophoneUsageDescription`.
# Under the hardened runtime a missing entitlement means no prompt and every
# request denied, with nothing else failing. The same check as
# `check_mac_microphone` in scripts/release.sh; prints each problem, returns 1
# if there is one.
MAC_MICROPHONE_USAGE="Biorouter uses the microphone only while you dictate, and does not save the recording."
mac_microphone_problems() { # <Biorouter.app>
  local app="$1" target usage bad=0
  for target in "$app" \
                "$app/Contents/Frameworks/Biorouter Helper.app" \
                "$app/Contents/Resources/bin/biorouter" \
                "$app/Contents/Resources/bin/biorouterd"; do
    if ! codesign -d --entitlements - --xml "$target" 2>/dev/null | python3 -c '
import plistlib, sys
try:
    entitlements = plistlib.loads(sys.stdin.buffer.read())
except Exception:
    sys.exit(1)
sys.exit(0 if entitlements.get("com.apple.security.device.audio-input") is True else 1)'; then
      printf 'com.apple.security.device.audio-input is not in the signed entitlements of %s\n' "$target"
      bad=1
    fi
  done
  usage="$(plutil -extract NSMicrophoneUsageDescription raw "$app/Contents/Info.plist" 2>/dev/null || true)"
  if [ "$usage" != "$MAC_MICROPHONE_USAGE" ]; then
    printf 'NSMicrophoneUsageDescription is "%s", expected "%s"\n' "$usage" "$MAC_MICROPHONE_USAGE"
    bad=1
  fi
  return "$bad"
}

smoke_mac() (
  local arch="$1" dmg="$2" mount="" tmp="" runner=(/usr/bin/env)
  local pid="" daemon_pid="" failure="" mounted=false
  mac_descendants() {
    local c
    for c in $(pgrep -P "$1" 2>/dev/null || true); do
      printf '%s\n' "$c"
      mac_descendants "$c"
    done
  }
  cleanup_mac() {
    local child tree=""
    if [ -n "$pid" ]; then
      tree="$(mac_descendants "$pid" | tr '\n' ' ')"
      kill -TERM "$pid" 2>/dev/null || true
      for _ in $(seq 1 40); do
        ps -o stat= -p "$pid" 2>/dev/null | grep -qv Z || break
        sleep 0.5
      done
      if ps -o stat= -p "$pid" 2>/dev/null | grep -qv Z; then
        [ -n "$failure" ] || failure="$arch desktop did not exit within 20 s of SIGTERM"
        kill -KILL "$pid" 2>/dev/null || true
      fi
      wait "$pid" 2>/dev/null || true
      for child in $tree; do
        [ "$child" = "$daemon_pid" ] && continue
        kill -KILL "$child" 2>/dev/null || true
      done
      pid=""
    fi
    if [ -n "$daemon_pid" ] && [ "$(ps -o comm= -p "$daemon_pid" 2>/dev/null | xargs basename 2>/dev/null)" = biorouterd ]; then
      kill -TERM "$daemon_pid" 2>/dev/null || true
      for _ in $(seq 1 30); do
        ps -o stat= -p "$daemon_pid" 2>/dev/null | grep -qv Z || break
        sleep 0.5
      done
      if ps -o stat= -p "$daemon_pid" 2>/dev/null | grep -qv Z; then
        [ -n "$failure" ] || failure="$arch shared daemon did not stop within 15 s"
        [ "$(ps -o comm= -p "$daemon_pid" 2>/dev/null | xargs basename 2>/dev/null)" != biorouterd ] \
          || kill -KILL "$daemon_pid" 2>/dev/null || true
      fi
      daemon_pid=""
    fi
    if [ "$mounted" = true ]; then
      if hdiutil detach "$mount" >/dev/null 2>&1 || {
        sleep 2
        hdiutil detach -force "$mount" >/dev/null
      }; then
        mounted=false
      else
        [ -n "$failure" ] || failure="$arch smoke mount did not detach"
      fi
    fi
    if [ -n "$failure" ] && [ -n "$tmp" ]; then
      if [ -f "$tmp/app.log" ]; then sed -n '1,160p' "$tmp/app.log" >&2 || true; fi
      if [ -f "${main_log:-}" ]; then sed -n '1,160p' "$main_log" >&2 || true; fi
    fi
    # Hermit can leave read-only package caches in the disposable HOME.
    if [ -n "$tmp" ]; then
      chmod -R u+w "$tmp" 2>/dev/null || true
      rm -rf "$tmp" 2>/dev/null || true
      tmp=""
    fi
    if [ -n "$mount" ] && [ "$mounted" = false ]; then
      chmod -R u+w "$mount" 2>/dev/null || true
      rm -rf "$mount" 2>/dev/null || true
      mount=""
    fi
  }
  trap 'smoke_status=$?; trap - EXIT; cleanup_mac; exit "$smoke_status"' EXIT
  require_file "$dmg"
  mount="$(mktemp -d "/tmp/biorouter-${arch}-mount.XXXXXX")"
  tmp="$(mktemp -d "/tmp/biorouter-${arch}-smoke.XXXXXX")"
  mkdir -m 700 "$tmp/userData"
  hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$dmg" >/dev/null
  mounted=true
  local app="$mount/Biorouter.app"
  [ -d "$app" ] || die "$arch DMG does not contain Biorouter.app"
  python3 "$ROOT/scripts/computer-use-runtime.py" verify "darwin-$arch" --directory "$app/Contents/Resources/computer-use" --require-signed
  codesign --verify --deep --strict --verbose=2 "$app"
  spctl --assess --type execute --verbose "$app"
  xcrun stapler validate "$app"
  local microphone=""
  microphone="$(mac_microphone_problems "$app")" || die "$arch microphone: $microphone"
  if [ "$arch" = x64 ]; then
    arch -x86_64 /usr/bin/true >/dev/null 2>&1 || die "Rosetta is required for Intel runtime verification"
    runner=(arch -x86_64)
  fi
  file "$app/Contents/Resources/bin/biorouter" | grep -q "${arch/x64/x86_64}" \
    || die "$arch CLI architecture mismatch"
  "${runner[@]}" "$app/Contents/Resources/bin/biorouter" --version | grep -q "$VERSION"
  "${runner[@]}" "$app/Contents/Resources/bin/biorouterd" --version | grep -q "$VERSION"
  check_dictation_size "darwin-$arch" "$app/Contents/Resources/bin/biorouter"
  check_dictation_size "darwin-$arch" "$app/Contents/Resources/bin/biorouterd"
  scan_ggml "$app" || die "$arch app bundle carries a speech model"
  # The local dictation engine loads and decodes in both shipped binaries (DD-12): Metal on
  # arm64. The Intel build runs under Rosetta, which hides AVX2 from CPUID unless
  # ROSETTA_ADVERTISE_AVX=1 asks it not to, and honours that only on macOS 15 or later.
  if [ "$arch" = arm64 ]; then
    bash "$ROOT/scripts/dictation-self-test.sh" --expect-backend metal \
      "$app/Contents/Resources/bin/biorouter" "$app/Contents/Resources/bin/biorouterd" \
      || die "$arch dictation self test failed"
  else
    local macos_major
    macos_major="$(sw_vers -productVersion | cut -d. -f1)"
    [ "$macos_major" -ge 15 ] || die "the Intel dictation self test needs macOS 15 or later on this Mac"
    bash "$ROOT/scripts/dictation-self-test.sh" --prefix "env ROSETTA_ADVERTISE_AVX=1 arch -x86_64" \
      --expect-backend cpu \
      "$app/Contents/Resources/bin/biorouter" "$app/Contents/Resources/bin/biorouterd" \
      || die "$arch dictation self test failed"
  fi
  # ⚠ **`--user-data-dir`, because HOME does not move it.** On macOS Electron resolves userData
  # through the system (~/Library/Application Support/Biorouter), not through $HOME, so without
  # it the smoke run wrote into the operator's real profile, beside an app they may be using.
  # The app sets no userData path of its own, and Electron honours the switch (measured on the
  # Linux build: main.log and the whole profile moved into the given directory).
  HOME="$tmp" BIOROUTER_DISABLE_KEYRING=true XDG_STATE_HOME="$tmp/.local/state" \
    /usr/bin/env -u BIOROUTER_PATH_ROOT -u BIOROUTER_DEV_PROFILE_ROOT "${runner[@]}" "$app/Contents/MacOS/Biorouter" --use-mock-keychain --user-data-dir="$tmp/userData" \
    >"$tmp/app.log" 2>&1 &
  pid=$!
  local main_log="$tmp/userData/logs/main.log" prompt=""
  local runtime="$tmp/.local/state/biorouter/daemon/runtime.json"
  local key="$tmp/.local/state/biorouter/daemon/user-action-key.json"
  for _ in $(seq 1 60); do
    prompt="$(pgrep -P "$pid" -x osascript 2>/dev/null | head -n 1 || true)"
    if [ -n "$prompt" ]; then
      failure="$arch desktop opened a startup prompt"
      break
    fi
    kill -0 "$pid" 2>/dev/null || break
    grep -qs 'Fatal error during startup' "$main_log" && break
    sleep 0.5
  done
  if [ -f "$runtime" ]; then
    daemon_pid="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["pid"])' "$runtime")"
  fi
  # A startup prompt found above is the failure to report: the checks below would read the
  # same blocked start as "did not become ready" and hide the prompt that caused it.
  if [ -n "$failure" ]; then
    :
  elif ! kill -0 "$pid" 2>/dev/null; then
    failure="$arch desktop exited during startup"
  elif grep -qs 'Fatal error during startup' "$main_log"; then
    failure="$arch desktop failed during startup"
  elif ! grep -qs 'React ready event received' "$main_log"; then
    failure="$arch desktop did not become ready within 30 s"
  else
    if ! python3 - "$runtime" "$key" <<'PYKEY'
import json, os, stat, sys
runtime_path, key_path = sys.argv[1:]
with open(runtime_path) as f:
    runtime = json.load(f)
with open(key_path) as f:
    key = json.load(f)
assert stat.S_IMODE(os.stat(os.path.dirname(key_path)).st_mode) == 0o700
assert stat.S_IMODE(os.stat(key_path).st_mode) == 0o600
assert all(key[field] == runtime[field] for field in ("profile_id", "instance_id", "pid"))
os.kill(runtime["pid"], 0)
PYKEY
    then
      failure="$arch desktop did not save a private matching daemon key"
    fi
  fi
  cleanup_mac
  trap - EXIT
  [ -z "$failure" ] || die "$failure"
  log "macOS $arch DMG, microphone entitlement and prompt sentence, CLI, daemon, dictation self test, silent shared startup, and desktop shutdown passed"
)

# Both packages use the same bounded silent-startup, replacement, fatal-error shutdown,
# and per-window checks. `timeout` also bounds the whole container run.
LINUX_DESKTOP_SMOKE="$ROOT/scripts/smoke-linux-desktop.sh"

# No speech model ships in any artifact (R6, PR-11). Two guards here, because each misses
# what the other catches:
#
# * a scan of every packaged file for the legacy ggml magic (`lmgg` on disk), which finds
#   a model file however it is named: on Linux every file the installed package lists
#   (`dpkg -L`, `rpm -ql`), on macOS the whole app bundle;
# * a size ceiling for `biorouter` and `biorouterd`, which finds a speech model compiled
#   into a binary with `include_bytes!`. A scan of the binaries cannot: the loader itself
#   holds the magic. The smallest speech model is 264 MB, so any of them blows the
#   ceiling. The 885,098 B voice detector does not, and is left to the source test
#   `no_model_is_embedded` in crates/biorouter-dictation, which finds it by its path.
#
# Each ceiling is the binary's shipped size in v1.94.1 (before the engine), plus the growth
# since, plus slack. On macOS arm64 the growth is measured [M]: this branch's stripped
# release build against v1.94.1 with its signature removed (164,049,820 B and 154,454,440
# B unsigned), +12,711,092 B for biorouter and +11,023,544 B for biorouterd at PR-12b
# (2026-11-07). That is PR-09's spike figure of 6.69 MB plus what the engine's spike left
# out, PR-10's supervisor, PR-11's installer and 352,078 B bench clip (+9,613,412 B and
# +8,306,152 B at PR-11), PR-12 to PERF-1's transcribe, dictionary and routes, which took
# biorouter to +11,613,732 B and biorouterd to +11,107,928 B without raising these lines
# (4ba336a3a, measured 2026-11-07: over the PR-11 ceilings by 611,804 B and 1,431,976 B
# before signing, so a release from it would have failed here), and PR-12b's AAC decoder,
# MP4 reader, Vorbis decoder and Ogg reader (O-39), +949,904 to +1,097,360 B to biorouter
# over three release builds as PR-12b grew (the first had neither the MPEG-2 ADTS reader
# nor the M4A trim, the last both, +982,992 B: builds of nearly the same tree differ by
# about 150 KB here, so the ceiling takes the largest), while
# biorouterd, which reads no file, moved by -84,384 B. Its slack is 1 MiB, which also
# holds the code signature's growth with the binary (339,940 B on v1.94.1's biorouter
# [M]). No Linux or Intel macOS build of this branch was measured, so their growth is
# inferred [I]: PR-09's figure for the target (Linux 2.17 MB, macOS x86_64 1.69 MB) plus
# the same non-spike growth as arm64 (biorouter 6,021,092 B, biorouterd 4,333,544 B), with
# 32 MiB of slack so an estimate off by a few MB does not fail the release; 32 MiB still
# hides no speech model. The four codecs alone add 1,115,736 B to a stripped Linux x86_64
# probe built in the release image [M, PR-12], 1,081,624 B to a macOS x86_64 probe and
# 1,048,096 B to a macOS arm64 one [M, PR-12b], so arm64's growth, which the inferred legs
# take, already holds them. Replace an inferred leg with a measured one, and its slack with
# 1 MiB, from the first CI cross build of these binaries. A pull request that grows other
# code raises its ceiling here, in the same change, and says why.
dictation_size_ceiling() { # <platform> <binary>
  local slack=1048576 inferred=$((32 << 20)) more_cli=6021092 more_daemon=4333544
  case "$1/$2" in
    linux-x64/biorouter) echo $((187413248 + 2170000 + more_cli + inferred)) ;;
    linux-x64/biorouterd) echo $((176920720 + 2170000 + more_daemon + inferred)) ;;
    darwin-arm64/biorouter) echo $((164389760 + 12711092 + slack)) ;;
    darwin-arm64/biorouterd) echo $((154775664 + 11023544 + slack)) ;;
    darwin-x64/biorouter) echo $((176538336 + 1690000 + more_cli + inferred)) ;;
    darwin-x64/biorouterd) echo $((166285184 + 1690000 + more_daemon + inferred)) ;;
    *) die "no dictation size ceiling for $1/$2" ;;
  esac
}

check_dictation_size() { # <platform> <binary path>
  local name size ceiling
  name="$(basename "$2")"
  size="$(wc -c <"$2" | tr -d ' ')"
  ceiling="$(dictation_size_ceiling "$1" "$name")"
  [ "$size" -le "$ceiling" ] \
    || die "$1 $name is $size bytes, over its ceiling of $ceiling: is a speech model compiled in?"
}

# Defined as text so the Linux containers run the same functions (eval "$GGML_SCAN").
# `scan_ggml <path>...` reads every file under the given paths and fails on a path that
# does not exist, so a renamed folder cannot make it scan nothing. `scan_ggml_list` reads
# a package's own file list on stdin (`dpkg -L <package>`, `rpm -ql <package>`), checks
# each regular file in it, and fails on an empty list. Both fail on the first file that
# starts with the ggml magic, and turn off `set -x` while they read, so a bundle of
# thousands of files does not flood the log.
GGML_SCAN='ggml_magic_at() {
  [ "$(head -c 4 "$1" | tr -d "\000")" = lmgg ]
}
scan_ggml() {
  local f p trace=$-
  set +x
  for p in "$@"; do
    if [ ! -e "$p" ]; then
      echo "scan_ggml: $p does not exist" >&2
      return 1
    fi
  done
  while IFS= read -r f; do
    if ggml_magic_at "$f"; then
      echo "a speech model file ships in the package: $f" >&2
      return 1
    fi
  done < <(find "$@" -type f)
  case $trace in *x*) set -x ;; esac
}
scan_ggml_list() {
  local f n=0 trace=$-
  set +x
  while IFS= read -r f; do
    [ -f "$f" ] && [ ! -L "$f" ] || continue
    n=$((n + 1))
    if ggml_magic_at "$f"; then
      echo "a speech model file ships in the package: $f" >&2
      return 1
    fi
  done
  if [ "$n" -eq 0 ]; then
    echo "scan_ggml_list: the package lists no files" >&2
    return 1
  fi
  case $trace in *x*) set -x ;; esac
}'
eval "$GGML_SCAN"

# The dictation self test inside a Linux container: the helper and the vendored fixtures
# mounted read only, and ROSETTA_ADVERTISE_AVX=1 for Docker Desktop's amd64 emulation on an
# Apple Silicon Mac (an x86 host ignores it). Run in the container as
#   bash /smoke/dictation-self-test.sh --fixtures /whisper.cpp/source --expect-backend cpu BIN...
DICTATION_SELF_TEST="$ROOT/scripts/dictation-self-test.sh"
dictation_container_args() {
  require_file "$DICTATION_SELF_TEST"
  require_file "$ROOT/vendor/whisper.cpp/source/samples/jfk.wav"
  DICTATION_DOCKER_ARGS=(-e ROSETTA_ADVERTISE_AVX=1
    -e "CEIL_BIOROUTER=$(dictation_size_ceiling linux-x64 biorouter)"
    -e "CEIL_BIOROUTERD=$(dictation_size_ceiling linux-x64 biorouterd)"
    -e "GGML_SCAN=$GGML_SCAN"
    -v "$ROOT/vendor/whisper.cpp":/whisper.cpp:ro
    -v "$DICTATION_SELF_TEST":/smoke/dictation-self-test.sh:ro)
}

smoke_deb() {
  local deb="$DESK/out/make/deb/x64/biorouter_${VERSION}_amd64.deb"
  require_file "$deb"
  require_file "$LINUX_DESKTOP_SMOKE"
  dictation_container_args
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    -v "$deb":/pkg/biorouter.deb:ro \
    -v "$LINUX_DESKTOP_SMOKE":/smoke/linux-desktop.sh:ro \
    "${DICTATION_DOCKER_ARGS[@]}" \
    debian:bookworm-slim bash -euxc '
      apt-get update -qq
      apt-get install -y -qq /pkg/biorouter.deb xvfb >/dev/null
      /usr/lib/biorouter/resources/bin/biorouter --version | grep -q "$VERSION"
      /usr/lib/biorouter/resources/bin/biorouterd --version | grep -q "$VERSION"
      test "$(stat -c %s /usr/lib/biorouter/resources/bin/biorouter)" -le "$CEIL_BIOROUTER"
      test "$(stat -c %s /usr/lib/biorouter/resources/bin/biorouterd)" -le "$CEIL_BIOROUTERD"
      eval "$GGML_SCAN"
      dpkg -L "$(dpkg-deb -f /pkg/biorouter.deb Package)" | scan_ggml_list
      bash /smoke/dictation-self-test.sh --fixtures /whisper.cpp/source --expect-backend cpu \
        /usr/lib/biorouter/resources/bin/biorouter /usr/lib/biorouter/resources/bin/biorouterd
      timeout -k 10 600 bash /smoke/linux-desktop.sh /usr/bin/biorouter
    '
  log "Linux desktop DEB, CLI, daemon, dictation self test, silent shared startup, replacement, fatal-error shutdown, and per-window startup passed"
}

smoke_rpm() {
  local rpm="$DESK/out/make/rpm/x64/Biorouter-${VERSION}-1.x86_64.rpm"
  require_file "$rpm"
  require_file "$LINUX_DESKTOP_SMOKE"
  dictation_container_args
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    -v "$rpm":/pkg/biorouter.rpm:ro \
    -v "$LINUX_DESKTOP_SMOKE":/smoke/linux-desktop.sh:ro \
    "${DICTATION_DOCKER_ARGS[@]}" \
    rockylinux:9 bash -euxc '
      dnf install -y -q /pkg/biorouter.rpm xorg-x11-server-Xvfb >/dev/null
      /usr/lib/Biorouter/resources/bin/biorouter --version | grep -q "$VERSION"
      /usr/lib/Biorouter/resources/bin/biorouterd --version | grep -q "$VERSION"
      test "$(stat -c %s /usr/lib/Biorouter/resources/bin/biorouter)" -le "$CEIL_BIOROUTER"
      test "$(stat -c %s /usr/lib/Biorouter/resources/bin/biorouterd)" -le "$CEIL_BIOROUTERD"
      eval "$GGML_SCAN"
      rpm -ql "$(rpm -qp --qf "%{NAME}" /pkg/biorouter.rpm)" | scan_ggml_list
      bash /smoke/dictation-self-test.sh --fixtures /whisper.cpp/source --expect-backend cpu \
        /usr/lib/Biorouter/resources/bin/biorouter /usr/lib/Biorouter/resources/bin/biorouterd
      timeout -k 10 600 bash /smoke/linux-desktop.sh /usr/bin/Biorouter
    '
  log "Linux desktop RPM, CLI, daemon, dictation self test, silent shared startup, replacement, fatal-error shutdown, and per-window startup passed"
}

smoke_cli_packages() {
  local deb="$ROOT/dist/cli/biorouter-cli_${VERSION}_amd64.deb"
  local rpm="$ROOT/dist/cli/biorouter-cli-${VERSION}-1.x86_64.rpm"
  require_file "$deb"
  require_file "$rpm"
  dictation_container_args
  # `biorouter dictation status --format json` from each container lands here and is
  # checked against schema 1 on this machine (PR-10).
  local status_dir
  status_dir="$(mktemp -d)"
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    "${DICTATION_DOCKER_ARGS[@]}" -v "$status_dir":/status \
    -v "$deb":/pkg/biorouter-cli.deb:ro debian:bookworm-slim bash -euxc '
      apt-get update -qq
      apt-get install -y -qq /pkg/biorouter-cli.deb >/dev/null
      biorouter --version | grep -q "$VERSION"
      biorouterd --version | grep -q "$VERSION"
      test "$(stat -c %s /usr/bin/biorouter)" -le "$CEIL_BIOROUTER"
      test "$(stat -c %s /usr/bin/biorouterd)" -le "$CEIL_BIOROUTERD"
      eval "$GGML_SCAN"
      dpkg -L "$(dpkg-deb -f /pkg/biorouter-cli.deb Package)" | scan_ggml_list
      bash /smoke/dictation-self-test.sh --fixtures /whisper.cpp/source --expect-backend cpu \
        "$(command -v biorouter)" "$(command -v biorouterd)"
      HOME=$(mktemp -d) biorouter dictation status --format json >/status/debian.json
      biorouter term --help >/dev/null
    '
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    "${DICTATION_DOCKER_ARGS[@]}" -v "$status_dir":/status \
    -v "$rpm":/pkg/biorouter-cli.rpm:ro rockylinux:9 bash -euxc '
      dnf install -y -q /pkg/biorouter-cli.rpm >/dev/null
      biorouter --version | grep -q "$VERSION"
      biorouterd --version | grep -q "$VERSION"
      test "$(stat -c %s /usr/bin/biorouter)" -le "$CEIL_BIOROUTER"
      test "$(stat -c %s /usr/bin/biorouterd)" -le "$CEIL_BIOROUTERD"
      eval "$GGML_SCAN"
      rpm -ql "$(rpm -qp --qf "%{NAME}" /pkg/biorouter-cli.rpm)" | scan_ggml_list
      bash /smoke/dictation-self-test.sh --fixtures /whisper.cpp/source --expect-backend cpu \
        "$(command -v biorouter)" "$(command -v biorouterd)"
      HOME=$(mktemp -d) biorouter dictation status --format json >/status/rocky.json
      biorouter term --help >/dev/null
    '
  local distro
  for distro in debian rocky; do
    python3 -I "$ROOT/scripts/check-dictation-status-json.py" --expect-capture-reason not_compiled \
      --expect-no-models "$status_dir/$distro.json" \
      || die "dictation status in the $distro CLI package is not schema 1"
  done
  rm -rf "$status_dir"
  log "CLI-only DEB/RPM version, dictation self test and status, binary size ceilings, no speech model, and terminal entry points passed"
}

smoke_serve() {
  # Replaces the retired tarball's smoke test, which asserted only that the
  # served page contained "<!doctype html>" -- true of a completely
  # non-functional app. This drives the real thing: install the CLI package,
  # run `biorouter serve`, and check the browser contract end to end.
  local deb="$ROOT/dist/cli/biorouter-cli_${VERSION}_amd64.deb"
  require_file "$deb"
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    -v "$deb":/pkg/biorouter-cli.deb:ro debian:bookworm-slim bash -euxc '
      apt-get update -qq
      apt-get install -y -qq /pkg/biorouter-cli.deb curl >/dev/null
      # The interface ships at the FHS location, because <exe dir>/../web from
      # /usr/bin would be /usr/web.
      test -s /usr/share/biorouter/web/index.html
      export HOME=/tmp/biorouter-home
      biorouter serve --host 127.0.0.1 --port 18080 --token smoketoken \
        >/tmp/serve.log 2>&1 &
      pid=$!
      for _ in $(seq 1 60); do
        curl -fsS -o /dev/null "http://127.0.0.1:18080/?t=smoketoken" && break
        sleep 1
      done

      # The document is gated. A bare GET must NOT hand out the shell -- it
      # carries the daemon secret.
      code=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:18080/)
      test "$code" = "401"

      # The token is exchanged for a session cookie. It is not consumed
      # (SD-9 in docs/deployment/serve-decisions.md): the readiness loop above
      # has already redeemed it. The answer is a page served by the daemon
      # itself that moves on to ./, not a 303, so the SameSite=Strict cookie
      # survives a navigation started by the file: page that serve --open writes.
      curl -s -o /tmp/bounce.html -D /tmp/h "http://127.0.0.1:18080/?t=smoketoken"
      grep -qi "^HTTP/1.1 200" /tmp/h
      grep -qi "set-cookie: biorouter_session=" /tmp/h
      grep -qi "HttpOnly" /tmp/h
      grep -qi "SameSite=Strict" /tmp/h
      grep -qi "cache-control: no-store" /tmp/h
      grep -q "http-equiv=\"refresh\" content=\"0;url=./\"" /tmp/bounce.html
      # `! grep` cannot fail a `set -e` script: bash exempts a negated
      # pipeline from errexit, so each absence is an explicit exit.
      if grep -q smoketoken /tmp/bounce.html; then exit 1; fi

      # And a wrong one is refused, so the gate is a gate.
      code=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:18080/?t=wrong")
      test "$code" = "401"

      # With the cookie the shell arrives, carrying the runtime configuration
      # the renderer reads. apiBaseUrl must be ABSENT: empty is falsy there and
      # would send the browser to a hardcoded 127.0.0.1:3000.
      curl -fsS -b "biorouter_session=smoketoken" http://127.0.0.1:18080/ >/tmp/shell.html
      grep -q "__BIOROUTER_HEADLESS_CONFIG__" /tmp/shell.html
      grep -q "\"secretKey\":\"[0-9a-f]\{64\}\"" /tmp/shell.html
      if grep -q "apiBaseUrl" /tmp/shell.html; then exit 1; fi

      # The bundle really is served, and it is the RELATIVE build, with the
      # mount named in the shell, so it also works under a reverse proxy path
      # prefix (issue #402).
      grep -q "src=\"./assets/" /tmp/shell.html
      grep -q "<head><base href=\"./\">" /tmp/shell.html
      asset=$(grep -o "/assets/index-[A-Za-z0-9_-]*\.js" /tmp/shell.html | head -1)
      curl -fsS -o /dev/null "http://127.0.0.1:18080$asset"

      # The API is on the same origin, still behind its own header, and the
      # interface endpoints are no longer unauthenticated.
      secret=$(grep -o "\"secretKey\":\"[0-9a-f]*\"" /tmp/shell.html | cut -d\" -f4)
      code=$(curl -s -o /dev/null -w "%{http_code}" http://127.0.0.1:18080/headless/health)
      test "$code" = "401"
      curl -fsS -H "X-Secret-Key: $secret" http://127.0.0.1:18080/headless/health >/tmp/health.json
      grep -q "\"status\":\"ok\"" /tmp/health.json

      # Stopping serve by pid stops its daemon: serve reaps it before exiting,
      # so the port is closed once wait returns. It used to stay bound by an
      # orphaned daemon that still honoured the token.
      kill "$pid"
      wait "$pid" || true
      if curl -s -o /dev/null --max-time 5 http://127.0.0.1:18080/status; then
        echo "the daemon outlived serve" >&2
        exit 1
      fi
    '
  log "biorouter serve: token exchange, gated shell, relative bundle and mount, authenticated endpoints, and stopping with its daemon passed"
}

case "$TARGET" in
  all)
    smoke_mac arm64 "$DESK/out/make/Biorouter-${VERSION}-arm64.dmg"
    smoke_mac x64 "$DESK/out/make/Biorouter-${VERSION}-x64.dmg"
    smoke_deb
    smoke_rpm
    smoke_cli_packages
    smoke_serve
    log "all locally executable release artifacts passed"
    ;;
  mac)
    smoke_mac arm64 "$DESK/out/make/Biorouter-${VERSION}-arm64.dmg"
    smoke_mac x64 "$DESK/out/make/Biorouter-${VERSION}-x64.dmg"
    ;;
  deb) smoke_deb ;;
  rpm) smoke_rpm ;;
  cli) smoke_cli_packages ;;
  serve) smoke_serve ;;
  *) die "unknown smoke target: $TARGET" ;;
esac
