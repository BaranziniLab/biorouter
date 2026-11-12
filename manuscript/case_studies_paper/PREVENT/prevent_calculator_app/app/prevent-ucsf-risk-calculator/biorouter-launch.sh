#!/usr/bin/env bash
# Shared launcher helpers for the exported Biorouter app "prevent-ucsf-risk-calculator".
# Sourced by run.sh / run.command; also usable on its own.
APP_ID="prevent-ucsf-risk-calculator"
DIR="${DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)}"
STORE="${XDG_CONFIG_HOME:-$HOME/.config}/biorouter/agent_drafter/$APP_ID"

die() { echo "error: $*" >&2; exit 1; }

# ── 1. Find biorouterd ────────────────────────────────────────────────────
# A bare `biorouterd` on PATH is the common case, but the desktop app ships its
# own copy and never puts it on PATH, so look there too. (The old launcher
# backgrounded a bare `biorouterd` inside `( ... & )`, which returns 0 even when
# the binary is missing, so `set -e` never caught it and the user just saw a
# dead page 40 seconds later.)
find_biorouterd() {
  # Guard optional vars with `:-`: run.sh runs under `set -u`, so a bare
  # $BIOROUTERD_BIN (normally UNSET, since biorouterd on PATH is the common case)
  # would abort the whole launcher with "unbound variable" before the PATH
  # lookup ever ran.
  if [ -n "${BIOROUTERD_BIN:-}" ] && [ -x "${BIOROUTERD_BIN:-}" ]; then echo "${BIOROUTERD_BIN}"; return 0; fi
  # A "fat" export bundles the daemon under payload/bin, so prefer it and keep the
  # folder is self-contained (no Biorouter install required on the target).
  if [ -x "$DIR/payload/bin/biorouterd" ]; then echo "$DIR/payload/bin/biorouterd"; return 0; fi
  if command -v biorouterd >/dev/null 2>&1; then command -v biorouterd; return 0; fi
  for p in \
    "$HOME/.local/bin/biorouterd" \
    "/usr/local/bin/biorouterd" \
    "/opt/homebrew/bin/biorouterd" \
    "/Applications/Biorouter.app/Contents/Resources/bin/biorouterd" \
    "$HOME/Applications/Biorouter.app/Contents/Resources/bin/biorouterd" \
    "/opt/Biorouter/resources/bin/biorouterd" \
    "/usr/lib/biorouter/resources/bin/biorouterd" \
    "/usr/lib/Biorouter/resources/bin/biorouterd"
  do
    [ -x "$p" ] && { echo "$p"; return 0; }
  done
  return 1
}

# ── 2. Install into the local Biorouter store ─────────────────────────────
# The daemon looks apps up by `manifest.json` in the store. Copying only the
# runtime files keeps the launcher/README/package.json out of the app dir, and
# re-syncing every run means `npm run build` edits show up on refresh.
install_app() {
  mkdir -p "$STORE"
  [ -f "$DIR/manifest.json" ] || die "manifest.json missing from this folder. Re-export the app."
  cp "$DIR/manifest.json" "$STORE/manifest.json"
  cp "$DIR/index.html" "$STORE/index.html" 2>/dev/null || true
  for sub in src dist assets; do
    if [ -d "$DIR/$sub" ]; then
      rm -rf "$STORE/$sub"
      cp -R "$DIR/$sub" "$STORE/$sub"
    fi
  done
}

port_alive() { curl -sf -o /dev/null --max-time 1 "http://127.0.0.1:$1/status" 2>/dev/null; }

# ── 3. A daemon whose secret this launcher knows ──────────────────────────
# A daemon serves an app's page only to a browser holding that app's access
# cookie, and the one-time link that sets the cookie is handed only to a caller
# holding the daemon's secret. So this launcher uses a daemon it started (its
# port and secret are kept, readable by you alone, in $LAUNCHER_STATE) or one you
# name with BIOROUTERD_PORT and BIOROUTER_SERVER__SECRET_KEY. It cannot open the
# app on any other daemon, whatever answers /status.
LAUNCHER_STATE="${XDG_CONFIG_HOME:-$HOME/.config}/biorouter/app-launcher/daemon"
SECRET=""
LOG=""

new_secret() { od -An -N32 -tx1 /dev/urandom | tr -d ' \n'; }

# Every request that carries the secret hands it to curl on stdin, never on a
# command line, where other accounts on this machine can read it.
secret_ok() {
  printf 'X-Secret-Key: %s\n' "$2" \
    | curl -sf -o /dev/null --max-time 2 -H @- "http://127.0.0.1:$1/apps" 2>/dev/null
}

# The path of this app's one-time launch link, from the daemon on :$1.
launch_path() {
  printf 'X-Secret-Key: %s\n' "$2" \
    | curl -sf --max-time 5 -X POST -H @- "http://127.0.0.1:$1/apps/$APP_ID/launch" 2>/dev/null \
    | sed -n 's|.*"path":"\(/apps/[A-Za-z0-9_-]*/?t=[0-9a-f]*\)".*|\1|p'
}

save_daemon() {
  ( umask 077
    mkdir -p "$(dirname "$LAUNCHER_STATE")"
    printf '%s %s\n' "$PORT" "$SECRET" > "$LAUNCHER_STATE.$$" && mv -f "$LAUNCHER_STATE.$$" "$LAUNCHER_STATE"
  ) || echo "  (could not remember this daemon; the next run starts another)" >&2
}

start_daemon() {
  if [ -n "${BIOROUTER_SERVER__SECRET_KEY:-}" ] && [ -n "${BIOROUTERD_PORT:-}" ] \
    && secret_ok "$BIOROUTERD_PORT" "$BIOROUTER_SERVER__SECRET_KEY"; then
    PORT="$BIOROUTERD_PORT"; SECRET="$BIOROUTER_SERVER__SECRET_KEY"
    echo "Using the Biorouter daemon on :$PORT"
    return 0
  fi
  if [ -f "$LAUNCHER_STATE" ]; then
    saved_port=""; saved_secret=""
    read -r saved_port saved_secret < "$LAUNCHER_STATE" || true
    if [ -n "$saved_port" ] && [ -n "$saved_secret" ] && secret_ok "$saved_port" "$saved_secret"; then
      PORT="$saved_port"; SECRET="$saved_secret"
      echo "Using the Biorouter daemon this launcher started on :$PORT"
      return 0
    fi
  fi

  BIOROUTERD="$(find_biorouterd)" || die "biorouterd not found. Install Biorouter, or set BIOROUTERD_BIN=/path/to/biorouterd"

  for p in "${BIOROUTERD_PORT:-3000}" 3001 3002 3003 3004 3005; do
    # A daemon already here is not one whose secret this launcher knows.
    if port_alive "$p"; then continue; fi
    # A new secret for every attempt: until the daemon answers with it, whatever
    # listens on the port is asked with it, and that must never be the secret of a
    # daemon that goes on to start somewhere else.
    SECRET="$(new_secret)"
    [ "${#SECRET}" -eq 64 ] || die "could not make a secret for the daemon (this needs od and /dev/urandom)"
    LOG="${TMPDIR:-/tmp}/biorouterd-$APP_ID-$p.log"
    echo "Starting $BIOROUTERD on :$p (using your configured Biorouter provider)..."
    BIOROUTER_SERVER__SECRET_KEY="$SECRET" BIOROUTER_PORT="$p" "$BIOROUTERD" agent </dev/null >"$LOG" 2>&1 &
    for _ in $(seq 1 40); do
      if secret_ok "$p" "$SECRET"; then PORT="$p"; save_daemon; return 0; fi
      kill -0 "$!" 2>/dev/null || break   # it died; try the next port
      sleep 0.5
    done
    echo "  :$p did not come up (see $LOG)" >&2
  done
  die "could not start biorouterd.${LOG:+ Last log: $LOG}"
}

# ── 4. The one-time address that opens this app ───────────────────────────
# Opening it sets the app's access cookie in your browser and lands on the app.
# It works once and for a few minutes; run the launcher again for a new one.
launch_url() {
  path="$(launch_path "$PORT" "$SECRET")"
  [ -n "$path" ] || die "the daemon on :$PORT does not serve '$APP_ID'. Is the store at $STORE?"
  LAUNCH_URL="http://127.0.0.1:$PORT$path"
}

# The link opens the app for whoever uses it FIRST, and a command line is
# readable by every account on this machine (`ps`, /proc/<pid>/cmdline) while
# `open` or `xdg-open` runs. So the link never goes on one: it is written into a
# page only you can read, which sends the browser on to it, and the opener is
# handed that page. Pages older than ten minutes hold expired links and are
# removed here.
LAUNCH_PAGES="${XDG_CONFIG_HOME:-$HOME/.config}/biorouter/app-launcher/open"

launch_page() {
  ( umask 077
    set -C
    mkdir -p "$LAUNCH_PAGES" || exit 1
    [ -d "$LAUNCH_PAGES" ] && [ ! -L "$LAUNCH_PAGES" ] && [ -O "$LAUNCH_PAGES" ] || exit 1
    chmod 700 "$LAUNCH_PAGES" || exit 1
    find "$LAUNCH_PAGES" -type f -name 'launch-*.html' -mmin +10 -exec rm -f {} + 2>/dev/null || true
    name="$(new_secret)"
    [ "${#name}" -eq 64 ] || exit 1
    page="$LAUNCH_PAGES/launch-${name:0:32}.html"
    printf '%s%s%s%s%s\n' \
      '<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer><meta http-equiv="refresh" content="0;url=' \
      "$1" '"><title>Opening Biorouter app</title><p>Opening the app. If nothing happens, <a href="' \
      "$1" '">open it here</a>. The address works once.</p>' > "$page" || exit 1
    printf '%s\n' "$page"
  )
}

open_url() {
  echo "Opening $APP_ID at http://127.0.0.1:$PORT/apps/$APP_ID/"
  token="${1#"http://127.0.0.1:$PORT/apps/$APP_ID/?t="}"
  # The digits and letters are listed, not written as ranges: macOS's bash 3.2
  # reads [0-9a-f] by the locale's collation under UTF-8, where it takes
  # upper-case letters too.
  case "$token" in
    "$1" | *[!0123456789abcdef]*) die "not a launch link for $APP_ID; refusing to open it" ;;
  esac
  [ "${#token}" -eq 64 ] || die "not a launch link for $APP_ID; refusing to open it"
  page="$(launch_page "$1")" || page=""
  opened=0
  if [ -n "$page" ]; then
    case "$(uname -s)" in
      Darwin) open "$page" && opened=1 ;;
      *) if command -v xdg-open >/dev/null 2>&1; then xdg-open "$page" && opened=1; fi ;;
    esac
  fi
  # This terminal is yours alone, so the link can be shown here: for a browser
  # that cannot read the page (a snap-packaged one cannot read hidden folders),
  # or for when none opened.
  if [ "$opened" = 1 ]; then
    echo "If the app does not appear, open this address instead (it works once): $1"
  else
    echo "Open this address in your browser (it works once): $1"
  fi
}

# ── 5. First-run payload install (full-mode exports only) ─────────────────
# A "full" export carries the app's server-side dependencies under payload/:
# knowledge bases (.brkb archives), skills (plain directory trees), and
# export.json (the audit manifest). On the FIRST run we import them into the
# recipient's store, guarded by a marker so re-runs are no-ops. A launcher-mode
# export has no payload/ dir, so this is a clean no-op there.
#
# Consent: the payload list is printed and an interactive y/N confirmation is
# required. Set BIOROUTER_EXPORT_YES=1 to skip the prompt (CI / headless).
install_payload() {
  [ -d "$DIR/payload" ] || return 0                     # launcher-mode export: nothing to install
  CFG="${XDG_CONFIG_HOME:-$HOME/.config}/biorouter"
  MARKER="$CFG/.apps-payload-installed-$APP_ID"
  [ -f "$MARKER" ] && return 0                          # already installed on a previous run

  # Collect what there is to install (knowledge bases + skills). A payload that
  # only carries a bundled daemon (payload/bin) has nothing to import here.
  kbs=""; skills=""
  if [ -d "$DIR/payload/knowledge" ]; then
    for f in "$DIR/payload/knowledge/"*.brkb; do [ -e "$f" ] && kbs="$kbs $f"; done
  fi
  if [ -d "$DIR/payload/skills" ]; then
    for d in "$DIR/payload/skills/"*/; do [ -e "$d" ] && skills="$skills $d"; done
  fi
  [ -z "$kbs" ] && [ -z "$skills" ] && return 0

  echo ""
  echo "This app ships a payload to install into your Biorouter (see payload/export.json):"
  for f in $kbs;    do echo "  - knowledge base: $(basename "$f")"; done
  for d in $skills; do echo "  - skill: $(basename "$d")"; done

  if [ "${BIOROUTER_EXPORT_YES:-}" != "1" ]; then
    printf "Install this payload now? [y/N] "
    read -r reply || reply=""
    case "$reply" in
      y|Y|yes|YES) ;;
      *) echo "Skipping payload install. Re-run to install it later."; return 0 ;;
    esac
  fi

  mkdir -p "$CFG/skills"
  # Knowledge bases: the `biorouter` CLI has no `knowledge import` subcommand as
  # of the 2026-07 tree, so we stage each .brkb under
  # ~/.config/biorouter/knowledge-imports/ and point the user at the Knowledge
  # panel's importer. If a future CLI grows `knowledge import`, prefer it.
  for f in $kbs; do
    if command -v biorouter >/dev/null 2>&1 && biorouter knowledge import --help >/dev/null 2>&1; then
      biorouter knowledge import "$f" && echo "  - imported $(basename "$f")" || echo "  ! import failed for $(basename "$f")"
    else
      mkdir -p "$CFG/knowledge-imports"
      cp "$f" "$CFG/knowledge-imports/"
      echo "  - staged $(basename "$f") in $CFG/knowledge-imports/ (import it from Biorouter's Knowledge panel)"
    fi
  done
  # Skills install straight into the skills dir the daemon already reads.
  for d in $skills; do
    name="$(basename "$d")"
    rm -rf "$CFG/skills/$name"
    cp -R "$d" "$CFG/skills/$name"
    echo "  - installed skill $name"
  done

  mkdir -p "$CFG"
  : > "$MARKER"
  echo "Payload installed."
}
