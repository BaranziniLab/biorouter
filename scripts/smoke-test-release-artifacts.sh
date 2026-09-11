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

smoke_mac() {
  local arch="$1" dmg="$2" mount tmp runner=(/usr/bin/env)
  require_file "$dmg"
  mount="$(mktemp -d "/tmp/biorouter-${arch}-mount.XXXXXX")"
  tmp="$(mktemp -d "/tmp/biorouter-${arch}-smoke.XXXXXX")"
  hdiutil attach -nobrowse -readonly -mountpoint "$mount" "$dmg" >/dev/null
  local app="$mount/Biorouter.app"
  [ -d "$app" ] || die "$arch DMG does not contain Biorouter.app"
  codesign --verify --deep --strict --verbose=2 "$app"
  spctl --assess --type execute --verbose "$app"
  xcrun stapler validate "$app"
  if [ "$arch" = x64 ]; then
    arch -x86_64 /usr/bin/true >/dev/null 2>&1 || die "Rosetta is required for Intel runtime verification"
    runner=(arch -x86_64)
  fi
  file "$app/Contents/Resources/bin/biorouter" | grep -q "${arch/x64/x86_64}" \
    || die "$arch CLI architecture mismatch"
  "${runner[@]}" "$app/Contents/Resources/bin/biorouter" --version | grep -q "$VERSION"
  "${runner[@]}" "$app/Contents/Resources/bin/biorouterd" --version | grep -q "$VERSION"
  # ⚠ **Give the throwaway HOME a keychain, or macOS interrupts whoever is at
  # the machine.** `BIOROUTER_DISABLE_KEYRING` only silences OUR keyring use;
  # Electron's own `safeStorage` still reaches for the Keychain. With HOME
  # pointed at an empty temp dir there is no `login.keychain-db`, and macOS puts
  # up a modal — "A keychain cannot be found to store Biorouter" — whose buttons
  # are Cancel and **Reset To Defaults**. A verification run must not put a
  # button that resets someone's keychain search list in front of them. This
  # happened to the operator during a real verify run.
  #
  # An empty keychain in the temp HOME satisfies the lookup and is thrown away
  # with the directory. Failure is not fatal: the smoke test's subject is the
  # artifact, not the keychain.
  mkdir -p "$tmp/Library/Keychains"
  security create-keychain -p "" "$tmp/Library/Keychains/login.keychain-db" 2>/dev/null || true
  HOME="$tmp" BIOROUTER_DISABLE_KEYRING=true \
    "${runner[@]}" "$app/Contents/MacOS/Biorouter" --disable-gpu >"$tmp/app.log" 2>&1 &
  local pid=$!
  sleep 12
  kill -0 "$pid" 2>/dev/null || { sed -n '1,160p' "$tmp/app.log" >&2; die "$arch desktop exited during startup"; }
  kill "$pid" 2>/dev/null || true
  sleep 1
  kill -KILL "$pid" 2>/dev/null || true
  pkill -KILL -f "$mount/Biorouter.app/Contents/" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  hdiutil detach "$mount" >/dev/null 2>&1 || {
    sleep 2
    hdiutil detach -force "$mount" >/dev/null
  }
  # ⚠ `chmod` first, and never let cleanup decide the verdict. The app runs with
  # HOME="$tmp", and anything it invokes that touches hermit writes a READ-ONLY
  # package cache in there — `rm -rf` then fails with "Permission denied" /
  # "Directory not empty", returns non-zero, and under `set -e` sinks the entire
  # verification even though every artifact passed. That happened: a green
  # release reported "verification failed" because it could not tidy up.
  chmod -R u+w "$mount" "$tmp" 2>/dev/null || true
  rm -rf "$mount" "$tmp" 2>/dev/null || true
  log "macOS $arch DMG, CLI, daemon, and desktop startup passed"
}

smoke_deb() {
  local deb="$DESK/out/make/deb/x64/biorouter_${VERSION}_amd64.deb"
  require_file "$deb"
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    -v "$deb":/pkg/biorouter.deb:ro debian:bookworm-slim bash -euxc '
      apt-get update -qq
      apt-get install -y -qq /pkg/biorouter.deb xvfb >/dev/null
      /usr/lib/biorouter/resources/bin/biorouter --version | grep -q "$VERSION"
      /usr/lib/biorouter/resources/bin/biorouterd --version | grep -q "$VERSION"
      HOME=/tmp/biorouter-home BIOROUTER_DISABLE_KEYRING=true \
        xvfb-run -a /usr/bin/biorouter --no-sandbox >/tmp/biorouter.log 2>&1 &
      pid=$!
      sleep 12
      kill -0 "$pid"
      kill "$pid" || true
      sleep 1
      kill -KILL "$pid" || true
      wait "$pid" || true
    '
  log "Linux desktop DEB, CLI, daemon, and Xvfb startup passed"
}

smoke_rpm() {
  local rpm="$DESK/out/make/rpm/x64/Biorouter-${VERSION}-1.x86_64.rpm"
  require_file "$rpm"
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    -v "$rpm":/pkg/biorouter.rpm:ro rockylinux:9 bash -euxc '
      dnf install -y -q /pkg/biorouter.rpm xorg-x11-server-Xvfb >/dev/null
      /usr/lib/Biorouter/resources/bin/biorouter --version | grep -q "$VERSION"
      /usr/lib/Biorouter/resources/bin/biorouterd --version | grep -q "$VERSION"
      Xvfb :99 -screen 0 1280x800x24 >/tmp/xvfb.log 2>&1 &
      xvfb=$!
      export DISPLAY=:99
      HOME=/tmp/biorouter-home BIOROUTER_DISABLE_KEYRING=true \
        /usr/bin/Biorouter --no-sandbox >/tmp/biorouter.log 2>&1 &
      pid=$!
      sleep 12
      kill -0 "$pid"
      kill "$pid" "$xvfb" || true
      sleep 1
      kill -KILL "$pid" "$xvfb" || true
      wait "$pid" || true
    '
  log "Linux desktop RPM, CLI, daemon, and Xvfb startup passed"
}

smoke_cli_packages() {
  local deb="$ROOT/dist/cli/biorouter-cli_${VERSION}_amd64.deb"
  local rpm="$ROOT/dist/cli/biorouter-cli-${VERSION}-1.x86_64.rpm"
  require_file "$deb"
  require_file "$rpm"
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    -v "$deb":/pkg/biorouter-cli.deb:ro debian:bookworm-slim bash -euxc '
      apt-get update -qq
      apt-get install -y -qq /pkg/biorouter-cli.deb >/dev/null
      biorouter --version | grep -q "$VERSION"
      biorouterd --version | grep -q "$VERSION"
      biorouter term --help >/dev/null
    '
  docker run --rm --platform linux/amd64 -e VERSION="$VERSION" \
    -v "$rpm":/pkg/biorouter-cli.rpm:ro rockylinux:9 bash -euxc '
      dnf install -y -q /pkg/biorouter-cli.rpm >/dev/null
      biorouter --version | grep -q "$VERSION"
      biorouterd --version | grep -q "$VERSION"
      biorouter term --help >/dev/null
    '
  log "CLI-only DEB/RPM version and terminal entry points passed"
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
      # has already redeemed it.
      curl -s -o /dev/null -D /tmp/h "http://127.0.0.1:18080/?t=smoketoken"
      grep -qi "^HTTP/1.1 303" /tmp/h
      grep -qi "set-cookie: biorouter_session=" /tmp/h
      grep -qi "HttpOnly" /tmp/h
      grep -qi "SameSite=Strict" /tmp/h

      # And a wrong one is refused, so the gate is a gate.
      code=$(curl -s -o /dev/null -w "%{http_code}" "http://127.0.0.1:18080/?t=wrong")
      test "$code" = "401"

      # With the cookie the shell arrives, carrying the runtime configuration
      # the renderer reads. apiBaseUrl must be ABSENT: empty is falsy there and
      # would send the browser to a hardcoded 127.0.0.1:3000.
      curl -fsS -b "biorouter_session=smoketoken" http://127.0.0.1:18080/ >/tmp/shell.html
      grep -q "__BIOROUTER_HEADLESS_CONFIG__" /tmp/shell.html
      grep -q "\"secretKey\":\"[0-9a-f]\{64\}\"" /tmp/shell.html
      ! grep -q "apiBaseUrl" /tmp/shell.html

      # The bundle really is served, and it is the ROOT-BASE build.
      grep -q "src=\"/assets/" /tmp/shell.html
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
  log "biorouter serve: token exchange, gated shell, root-base bundle, authenticated endpoints, and stopping with its daemon passed"
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
