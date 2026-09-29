#!/usr/bin/env bash
# Runs INSIDE the DEB and RPM smoke containers; scripts/smoke-test-release-artifacts.sh mounts it.
# One script for both packages, so the two checks cannot drift apart.
#
# It launches the installed desktop app under Xvfb and checks what a first launch really does,
# not merely that a process is alive after a few seconds (which is also true of an app stuck at
# a prompt or an error box):
#
#   1. Shared daemon (the default): the app must be waiting at the zenity approval prompt, and
#      SIGTERM must end it and its prompt within a bounded time.
#   2. Same path, prompt closed without an answer: the app reports a fatal startup error and
#      must still exit on SIGTERM while that error is on screen. 1.92.0 sat in a synchronous
#      error box that ignored SIGTERM and hung verify.
#   3. Per-window daemon (BIOROUTER_SHARED_DAEMON=0): biorouterd must start and the window must
#      load, and SIGTERM must shut both down.
#
# Every wait is bounded. On any failure it prints the app's logs and exits non-zero.
#
# LANG and LC_ALL are left unset on purpose. That is the C locale 1.92.0 failed in: zenity could
# not convert the prompt's non-ASCII text, exited 255, and the app called it a cancellation.
set -euo pipefail

APP="${1:?usage: smoke-linux-desktop.sh <desktop launcher>}"
unset LANG LC_ALL
export DISPLAY=:99
BIOROUTER_DISABLE_KEYRING=true
export BIOROUTER_DISABLE_KEYRING

say() { printf '[linux-desktop-smoke] %s\n' "$*"; }

# The pids this script started, killed on exit whatever happens. Only these are ever killed.
started=()
home=""
app_log=""
cleanup() {
  local p
  for p in "${started[@]}"; do kill -KILL "$p" 2>/dev/null || true; done
}
trap cleanup EXIT

dump_logs() {
  local main_log
  if [ -n "$app_log" ] && [ -f "$app_log" ]; then
    say "app output ($app_log):" >&2
    sed -n '1,200p' "$app_log" >&2
  fi
  if [ -n "$home" ]; then
    main_log="$(find "$home" -name main.log -print -quit 2>/dev/null || true)"
    if [ -n "$main_log" ]; then
      say "main.log ($main_log):" >&2
      sed -n '1,200p' "$main_log" >&2
    fi
  fi
}

fail() {
  say "FAIL: $*" >&2
  dump_logs
  exit 1
}

# Process helpers read /proc directly: debian:bookworm-slim has no procps.
state_of() {
  local stat
  stat="$(cat "/proc/$1/stat" 2>/dev/null)" || return 1
  stat="${stat##*) }"
  printf '%s\n' "${stat%% *}"
}
ppid_of() {
  local stat
  stat="$(cat "/proc/$1/stat" 2>/dev/null)" || return 1
  stat="${stat##*) }"
  stat="${stat#* }"
  printf '%s\n' "${stat%% *}"
}
# Exited, or a zombie nobody has reaped yet: either way it no longer runs.
gone() {
  local state
  state="$(state_of "$1")" || return 0
  [ "$state" = Z ]
}
alive() { ! gone "$1"; }

# Every live descendant of $1 whose command name is $2, one pid per line.
descendants_named() {
  local root="$1" name="$2" changed=1 entry pid parent
  local -A tree=(["$root"]=1)
  while [ "$changed" = 1 ]; do
    changed=0
    for entry in /proc/[0-9]*; do
      pid="${entry#/proc/}"
      [ -n "${tree[$pid]:-}" ] && continue
      parent="$(ppid_of "$pid")" || continue
      if [ -n "${tree[$parent]:-}" ]; then
        tree["$pid"]=1
        changed=1
      fi
    done
  done
  for pid in "${!tree[@]}"; do
    [ "$pid" = "$root" ] && continue
    alive "$pid" || continue
    [ "$(cat "/proc/$pid/comm" 2>/dev/null)" = "$name" ] && printf '%s\n' "$pid"
  done
  return 0
}

# wait_for SECONDS COMMAND...: poll until COMMAND succeeds; non-zero once SECONDS have passed.
wait_for() {
  local deadline=$((SECONDS + $1))
  shift
  until "$@"; do
    [ "$SECONDS" -lt "$deadline" ] || return 1
    sleep 0.2
  done
}

main_log_has() {
  local main_log
  main_log="$(find "$home" -name main.log -print -quit 2>/dev/null || true)"
  [ -n "$main_log" ] && grep -q -- "$1" "$main_log"
}

launch() {
  local label="$1"
  shift
  home="/tmp/biorouter-home-$label"
  app_log="/tmp/biorouter-$label.log"
  rm -rf "$home"
  mkdir -p "$home"
  env HOME="$home" "$@" "$APP" --no-sandbox >"$app_log" 2>&1 &
  app_pid=$!
  started+=("$app_pid")
  say "$label: started the app as pid $app_pid"
}

# stop_app SECONDS: SIGTERM the app and require it to exit within SECONDS.
stop_app() {
  kill -TERM "$app_pid" 2>/dev/null || true
  wait_for "$1" gone "$app_pid" || fail "the app (pid $app_pid) did not exit within $1 s of SIGTERM"
  local status=0
  wait "$app_pid" || status=$?
  say "the app exited after SIGTERM (status $status)"
}

have_prompt() {
  prompt_pid="$(descendants_named "$app_pid" zenity)"
  prompt_pid="${prompt_pid%%$'\n'*}"
  [ -n "$prompt_pid" ]
}
app_gone_or_fatal() { gone "$app_pid" || main_log_has 'Fatal error during startup'; }
prompt_or_failure() { have_prompt || app_gone_or_fatal; }

wait_for_prompt() {
  wait_for 45 prompt_or_failure || fail "no approval prompt appeared within 45 s"
  if ! have_prompt; then
    gone "$app_pid" && fail "the app exited before showing the approval prompt"
    fail "startup failed before the approval prompt"
  fi
  started+=("$prompt_pid")
  say "the approval prompt is open (zenity pid $prompt_pid)"
  # A prompt that could not open exits at once (1.92.0: exit 255 after about 0.2 s).
  sleep 3
  alive "$prompt_pid" || fail "the approval prompt closed by itself"
  alive "$app_pid" || fail "the app exited while its approval prompt was open"
  if main_log_has 'Fatal error during startup'; then
    fail "startup failed while the approval prompt was open"
  fi
}

# Xvfb first, and wait until it accepts connections rather than guessing a delay.
Xvfb :99 -screen 0 1280x800x24 -nolisten tcp >/tmp/xvfb.log 2>&1 &
xvfb_pid=$!
started+=("$xvfb_pid")
xvfb_ready() { [ -S /tmp/.X11-unix/X99 ] || gone "$xvfb_pid"; }
wait_for 20 xvfb_ready || fail "Xvfb did not start within 20 s"
if gone "$xvfb_pid"; then
  cat /tmp/xvfb.log >&2
  fail "Xvfb exited during startup"
fi
say "Xvfb is ready on $DISPLAY"

# 1. Default first launch: the shared daemon asks for an approval secret before it starts.
launch shared-prompt
wait_for_prompt
stop_app 20
wait_for 10 gone "$prompt_pid" \
  || fail "the approval prompt (zenity pid $prompt_pid) outlived the app by 10 s"
say "shared daemon: the app waited at its approval prompt and quit on SIGTERM, closing the prompt"

# 2. Same path, with the prompt closed without an answer: the fatal startup error must not
#    leave a process that ignores SIGTERM.
launch shared-cancelled
wait_for_prompt
kill -TERM "$prompt_pid" 2>/dev/null || true
fatal_logged() { main_log_has 'Fatal error during startup' || gone "$app_pid"; }
wait_for 20 fatal_logged || fail "closing the approval prompt did not end startup within 20 s"
main_log_has 'Shared daemon startup cancelled' \
  || fail "closing the approval prompt was not reported as a cancellation"
alive "$app_pid" || fail "the app exited instead of reporting that startup was cancelled"
# Give the error dialog time to open, so SIGTERM arrives while it is on screen.
sleep 2
stop_app 20
say "shared daemon: after the prompt was closed the app reported it, and quit on SIGTERM"

# 3. Per-window daemon: biorouterd starts and the window loads.
launch per-window BIOROUTER_SHARED_DAEMON=0
have_daemon() {
  daemon_pid="$(descendants_named "$app_pid" biorouterd)"
  daemon_pid="${daemon_pid%%$'\n'*}"
  [ -n "$daemon_pid" ] || gone "$app_pid"
}
wait_for 60 have_daemon || fail "biorouterd did not start within 60 s"
gone "$app_pid" && fail "the app exited during startup"
started+=("$daemon_pid")
say "per-window daemon: biorouterd is running (pid $daemon_pid)"
window_ready() { main_log_has 'React ready event received' || app_gone_or_fatal; }
wait_for 90 window_ready || fail "the window did not finish loading within 90 s"
main_log_has 'Fatal error during startup' && fail "startup failed"
gone "$app_pid" && fail "the app exited during startup"
say "per-window daemon: the window loaded"
stop_app 20
wait_for 15 gone "$daemon_pid" || fail "biorouterd (pid $daemon_pid) outlived the app by 15 s"
say "per-window daemon: the app and biorouterd shut down on SIGTERM"

kill -TERM "$xvfb_pid" 2>/dev/null || true
wait "$xvfb_pid" 2>/dev/null || true
say "all desktop startup checks passed"
