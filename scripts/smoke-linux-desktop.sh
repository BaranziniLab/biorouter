#!/usr/bin/env bash
# Runs INSIDE the DEB and RPM smoke containers; scripts/smoke-test-release-artifacts.sh mounts it.
# One script for both packages, so the two checks cannot drift apart.
#
# Checks silent shared startup, reuse, missing-key replacement, fatal-error shutdown,
# and per-window startup under Xvfb. Every wait is bounded; LANG stays unset.
set -euo pipefail

APP="${1:?usage: smoke-linux-desktop.sh <desktop launcher>}"
unset LANG LC_ALL
export DISPLAY=:99
BIOROUTER_DISABLE_KEYRING=true
export BIOROUTER_DISABLE_KEYRING
chromium_args=()
if [ "$(id -u)" = 0 ] || [ "${BR_SMOKE_DISABLE_CHROMIUM_SANDBOX:-0}" = 1 ]; then
  chromium_args+=(--no-sandbox)
fi

say() { printf '[linux-desktop-smoke] %s\n' "$*"; }

# The pids this script started, killed on exit whatever happens. Only these are ever killed.
started=()
home=""
app_log=""
daemon_records=()
runtime_records=()
cleanup() {
  local p
  for p in "${started[@]}"; do kill -KILL "$p" 2>/dev/null || true; done
  local runtime
  for runtime in "${runtime_records[@]}"; do
    [ -f "$runtime" ] || continue
    daemon_records+=("$(json_field "$runtime" pid)")
  done
  for p in "${daemon_records[@]}"; do
    [ "$(cat "/proc/$p/comm" 2>/dev/null)" = biorouterd ] || continue
    kill -TERM "$p" 2>/dev/null || true
    wait_for 15 gone "$p" || kill -KILL "$p" 2>/dev/null || true
  done
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

json_field() {
  sed -nE 's/.*"'"$2"'"[[:space:]]*:[[:space:]]*"?([^",}[:space:]]+)"?.*/\1/p' "$1"
}

launch() {
  local label="$1"
  shift
  home="/tmp/biorouter-home-$label"
  app_log="/tmp/biorouter-$label.log"
  mkdir -p "$home/userData"
  chmod 700 "$home" "$home/userData"
  rm -f "$home/userData/logs/main.log"
  env -u BIOROUTER_PATH_ROOT -u BIOROUTER_DEV_PROFILE_ROOT -u XDG_STATE_HOME HOME="$home" "$@" \
    "$APP" "${chromium_args[@]}" --user-data-dir="$home/userData" >"$app_log" 2>&1 &
  app_pid=$!
  started+=("$app_pid")
  runtime_records+=("$home/.local/state/biorouter/daemon/runtime.json")
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

assert_no_prompt() {
  [ -z "$(descendants_named "$app_pid" zenity)" ] || fail "startup opened a zenity prompt"
}
app_gone_or_fatal() { gone "$app_pid" || main_log_has 'Fatal error during startup'; }
window_ready() {
  assert_no_prompt
  main_log_has 'React ready event received' || app_gone_or_fatal
}
require_ready() {
  wait_for "$1" window_ready || fail "the window did not finish loading within $1 s"
  main_log_has 'Fatal error during startup' && fail "startup failed"
  gone "$app_pid" && fail "the app exited during startup"
  assert_no_prompt
}
require_key() {
  local runtime="$home/.local/state/biorouter/daemon/runtime.json"
  local key="$home/.local/state/biorouter/daemon/user-action-key.json"
  [ -f "$runtime" ] && [ -f "$key" ] || fail "shared daemon did not save its runtime and key"
  [ "$(stat -c %a "${key%/*}")" = 700 ] || fail "daemon directory is not mode 700"
  [ "$(stat -c %a "$key")" = 600 ] || fail "daemon key is not mode 600"
  daemon_pid="$(json_field "$runtime" pid)"
  daemon_instance="$(json_field "$runtime" instance_id)"
  [ -n "$daemon_pid" ] && [ -n "$daemon_instance" ] || fail "invalid daemon identity"
  [ "$(json_field "$key" instance_id)" = "$daemon_instance" ] || fail "key instance does not match"
  [ "$(json_field "$key" profile_id)" = "$(json_field "$runtime" profile_id)" ] || fail "key profile does not match"
  [ "$(json_field "$key" pid)" = "$daemon_pid" ] || fail "key pid does not match"
  [ "$(cat "/proc/$daemon_pid/comm" 2>/dev/null)" = biorouterd ] || fail "shared daemon is not running"
  daemon_records+=("$daemon_pid")
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

# First launch and relaunch must reach the window without a prompt.
rm -rf /tmp/biorouter-home-shared
launch shared
require_ready 90
require_key
original_pid="$daemon_pid"
original_instance="$daemon_instance"
stop_app 20
alive "$original_pid" || fail "shared daemon did not survive app quit"
launch shared
require_ready 45
require_key
[ "$daemon_pid" = "$original_pid" ] && [ "$daemon_instance" = "$original_instance" ] \
  || fail "relaunch replaced a usable shared daemon"
stop_app 20
say "shared daemon: silent first launch and reuse passed"

# A missing key requires a verified replacement after the starter grace period.
rm "$home/.local/state/biorouter/daemon/user-action-key.json"
launch shared
require_ready 45
require_key
[ "$daemon_pid" != "$original_pid" ] && [ "$daemon_instance" != "$original_instance" ] \
  || fail "missing key did not replace the daemon"
wait_for 3 gone "$original_pid" || fail "old daemon survived replacement"
main_log_has 'Replacing the running background service' || fail "replacement was not reported"
stop_app 20
say "shared daemon: missing-key replacement passed"

# Startup failure must still accept SIGTERM while its error box is open.
rm -rf /tmp/biorouter-home-fatal
mkdir -p /tmp/biorouter-home-fatal/.local/state/biorouter/daemon
chmod 755 /tmp/biorouter-home-fatal/.local/state/biorouter/daemon
launch fatal
fatal_logged() { assert_no_prompt; main_log_has 'Fatal error during startup' || gone "$app_pid"; }
wait_for 30 fatal_logged || fail "insecure directory did not produce a startup error"
main_log_has 'Fatal error during startup' || fail "startup error was not logged"
alive "$app_pid" || fail "app exited before showing its startup error"
sleep 2
stop_app 20
say "fatal startup error: bounded SIGTERM passed"

# 3. Per-window daemon: biorouterd starts and the window loads.
rm -rf /tmp/biorouter-home-per-window
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
require_ready 90
say "per-window daemon: the window loaded"
stop_app 20
wait_for 15 gone "$daemon_pid" || fail "biorouterd (pid $daemon_pid) outlived the app by 15 s"
say "per-window daemon: the app and biorouterd shut down on SIGTERM"

kill -TERM "$xvfb_pid" 2>/dev/null || true
wait "$xvfb_pid" 2>/dev/null || true
say "all desktop startup checks passed"
