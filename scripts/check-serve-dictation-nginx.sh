#!/usr/bin/env bash
# PR-21 live evidence: HTTPS prefix, nginx body buffering, browser capture, and 413 halving.
# This script retains its sandbox, logs, model files, and stopped nginx container.
set -euo pipefail
set +x
umask 077

WT=/Users/wgu/Desktop/BioRouter/.claude/worktrees/local-dictation
UI="$WT/ui/desktop"
HX=/Users/wgu/biorouter-runs/dictation-2026-11/hx.sh
BASE=/Users/wgu/biorouter-runs/dictation-2026-11/sandboxes
CLI="$WT/target/debug/biorouter"
WEB="$UI/src/web"
SANDBOX=''
MODEL_CACHE=''
WAV=''
SERVE_PORT=18876
HTTPS_PORT=18443
HALVE_PORT=18444
BUILD_WEB=0
CREDENTIALS=/Users/wgu/Desktop/BioRouter/live-test.env

usage() {
  cat >&2 <<'EOF'
usage: check-serve-dictation-nginx.sh --sandbox-root ABS --model-cache DIR --wav PUBLIC_16KHZ_WAV
       [--serve-bin ABS] [--web-dir ABS]
       [--serve-port N] [--https-port N] [--halve-port N] [--build-web]
The sandbox must be a new direct child of /Users/wgu/biorouter-runs/dictation-2026-11/sandboxes.
EOF
  exit 2
}

fail() { echo "PR-21 nginx prerequisite or check failed: $*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
  case "$1" in
    --sandbox-root) [ "$#" -ge 2 ] || usage; SANDBOX="$2"; shift 2 ;;
    --model-cache) [ "$#" -ge 2 ] || usage; MODEL_CACHE="$2"; shift 2 ;;
    --wav) [ "$#" -ge 2 ] || usage; WAV="$2"; shift 2 ;;
    --serve-bin) [ "$#" -ge 2 ] || usage; CLI="$2"; shift 2 ;;
    --web-dir) [ "$#" -ge 2 ] || usage; WEB="$2"; shift 2 ;;
    --serve-port) [ "$#" -ge 2 ] || usage; SERVE_PORT="$2"; shift 2 ;;
    --https-port) [ "$#" -ge 2 ] || usage; HTTPS_PORT="$2"; shift 2 ;;
    --halve-port) [ "$#" -ge 2 ] || usage; HALVE_PORT="$2"; shift 2 ;;
    --build-web) BUILD_WEB=1; shift ;;
    *) usage ;;
  esac
done

[ -n "$SANDBOX" ] && [ -n "$MODEL_CACHE" ] && [ -n "$WAV" ] || usage
case "$SANDBOX" in "$BASE"/*) ;; *) fail "sandbox root must be under $BASE" ;; esac
RUN_NAME="${SANDBOX##*/}"
[[ "$RUN_NAME" =~ ^[a-zA-Z0-9][a-zA-Z0-9_-]*$ ]] && [ "${#RUN_NAME}" -le 40 ] || \
  fail 'sandbox root must be a direct named child of at most 40 characters'
[ "$SANDBOX" = "$BASE/$RUN_NAME" ] || fail 'sandbox root must be a direct child, with no traversal'
[ ! -e "$SANDBOX" ] && [ ! -L "$SANDBOX" ] || fail "sandbox already exists; choose a fresh name and preserve $SANDBOX"
for port in "$SERVE_PORT" "$HTTPS_PORT" "$HALVE_PORT"; do
  [[ "$port" =~ ^[0-9]+$ ]] && [ "$port" -ge 1024 ] && [ "$port" -le 65535 ] || fail "invalid port: $port"
done
[ "$SERVE_PORT" != "$HTTPS_PORT" ] && [ "$SERVE_PORT" != "$HALVE_PORT" ] \
  && [ "$HTTPS_PORT" != "$HALVE_PORT" ] || fail 'ports must be distinct'
[ -f "$MODEL_CACHE/ggml-small.en-q8_0.bin" ] \
  && [ -f "$MODEL_CACHE/ggml-silero-v6.2.0.bin" ] || \
  fail "cached small.en and Silero files missing under $MODEL_CACHE; supply the existing model-cache directory, do not download"
MODEL_CACHE="$(cd "$MODEL_CACHE" && pwd -P)"
[ -f "$WAV" ] || fail "public 16 kHz WAV missing: $WAV"
[ -f "$CREDENTIALS" ] && [ ! -L "$CREDENTIALS" ] && [ -r "$CREDENTIALS" ] || \
  fail 'the approved Versa credential file is missing or unreadable'
[ -x "$CLI" ] && [ -x "$(dirname "$CLI")/biorouterd" ] || \
  fail "paired biorouter and biorouterd binaries are missing; build them via bash $HX cargo build -p biorouter-cli -p biorouter-server"
[ -d "$UI/node_modules/@playwright/test" ] || \
  fail "Playwright package is missing under $UI/node_modules; prepare it without npm install/ci in this worktree"
command -v docker >/dev/null || fail 'Docker CLI unavailable; install/start Docker Desktop without restarting an owner daemon'
docker info >/dev/null 2>&1 || fail 'Docker daemon unavailable; start Docker Desktop without restarting an owner daemon'
docker image inspect docker.io/library/nginx:stable >/dev/null 2>&1 || fail 'docker.io/library/nginx:stable image missing; pull it explicitly before this run'
command -v openssl >/dev/null || fail 'openssl unavailable for the local HTTPS certificate'
command -v curl >/dev/null || fail 'curl unavailable for the HTTP probes'
command -v rg >/dev/null || fail 'rg unavailable for the worklet asset check'

if [ "$BUILD_WEB" -eq 1 ]; then
  (cd "$UI" && bash "$HX" npm run build:web) || fail "web bundle build failed; see $UI"
fi
[ -f "$WEB/index.html" ] || fail "web bundle missing at $WEB; run (cd $UI && bash $HX npm run build:web)"
WORKLET="$(find "$WEB/assets" -maxdepth 1 -type f -name '*pcmCapture*.js' -print -quit 2>/dev/null || true)"
[ -n "$WORKLET" ] || fail "worklet asset missing in $WEB/assets; rebuild web bundle via bash $HX npm run build:web"
rg -l --fixed-strings --glob "!${WORKLET##*/}" "${WORKLET##*/}" "$WEB/assets" >/dev/null || \
  fail "entry chunks do not reference ${WORKLET##*/}; check ?url&no-inline build output"

python3 - "$WAV" "$SERVE_PORT" "$HTTPS_PORT" "$HALVE_PORT" <<'PY'
import socket, sys, wave
with wave.open(sys.argv[1], 'rb') as wav:
    if wav.getframerate() != 16000 or wav.getnchannels() != 1:
        raise SystemExit('PR-21 nginx prerequisite: --wav must be mono 16 kHz PCM')
for raw in sys.argv[2:]:
    with socket.socket() as sock:
        try:
            sock.bind(('127.0.0.1', int(raw)))
        except OSError as error:
            raise SystemExit(f'PR-21 nginx prerequisite: port {raw} unavailable: {error}')
PY

mkdir -p "$SANDBOX/config" "$SANDBOX/data/sessions" "$SANDBOX/state" "$SANDBOX/tls" \
  "$SANDBOX/model-stage" "$SANDBOX/home" "$SANDBOX/temp"
printf '%s\n' 'BIOROUTER_PROVIDER: versa_azure' \
  'BIOROUTER_MODEL: gpt-5.5-2026-04-24' > "$SANDBOX/config/config.yaml"
for name in sessions.db sessions.db-wal sessions.db-shm; do
  [ ! -e "$SANDBOX/data/sessions/$name" ] || fail "session database unexpectedly seeded: $name"
done
printf '%s\n' "$WT" > "$SANDBOX/worktree"
printf '%s\n' "$WAV" > "$SANDBOX/public-wav-path"
printf '%s\n' "$MODEL_CACHE" > "$SANDBOX/model-cache-source-path"
for name in ggml-small.en-q8_0.bin ggml-silero-v6.2.0.bin; do
  ln -s "$MODEL_CACHE/$name" "$SANDBOX/model-stage/$name"
done
sysctl -n vm.loadavg > "$SANDBOX/load-before.txt" || fail 'could not record host load average'
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$SANDBOX/tls/key.pem" \
  -out "$SANDBOX/tls/cert.pem" -days 1 -subj '/CN=localhost' > "$SANDBOX/openssl.log" 2>&1 || \
  fail "local TLS certificate generation failed; see $SANDBOX/openssl.log"
chmod 600 "$SANDBOX/tls/key.pem"
RUN_ENV=(
  "BIOROUTER_PATH_ROOT=$SANDBOX"
  'BIOROUTER_DISABLE_KEYRING=true'
  "HOME=$SANDBOX/home"
  "XDG_CONFIG_HOME=$SANDBOX/home/.config"
  "XDG_DATA_HOME=$SANDBOX/home/.local/share"
  "XDG_STATE_HOME=$SANDBOX/home/.local/state"
  "TMPDIR=$SANDBOX/temp"
)
load_versa_credential() {
  local key
  key="$(
    set +x
    unset VERSA_AZURE_API_KEY
    set -a
    source "$CREDENTIALS" >/dev/null 2>&1 || exit 1
    printf '%s' "${VERSA_AZURE_API_KEY-}"
  )" 2>/dev/null || fail 'could not load the approved Versa credential file'
  [ -n "$key" ] || fail 'the approved Versa credential file has no Versa API key'
  unset BIOROUTER_PROVIDER BIOROUTER_MODEL BIOROUTER_LEAD_PROVIDER BIOROUTER_LEAD_MODEL \
    BIOROUTER_WORKER_PROVIDER BIOROUTER_WORKER_MODEL VERSA_AZURE_ENDPOINT \
    VERSA_AZURE_DEPLOYMENT_NAME VERSA_AZURE_API_VERSION BIOROUTER_PRIVACY_TEST_AUTH
  export VERSA_AZURE_API_KEY="$key"
}

ps_field() { ps -p "$1" -o "$2=" 2>/dev/null | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' || true; }
cwd_of() { lsof -a -p "$1" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p' | head -1 || true; }
record_pid() {
  local role="$1" pid="$2"
  printf '%s\n%s\n%s\n' "$pid" "$(ps_field "$pid" lstart)" "$(cwd_of "$pid")" > "$SANDBOX/$role.pid-record"
}
owns_pid() {
  local role="$1" pid started cwd comm
  [ -f "$SANDBOX/$role.pid-record" ] || return 1
  pid="$(sed -n '1p' "$SANDBOX/$role.pid-record")"
  started="$(sed -n '2p' "$SANDBOX/$role.pid-record")"
  cwd="$(sed -n '3p' "$SANDBOX/$role.pid-record")"
  [[ "$pid" =~ ^[0-9]+$ ]] && [ -n "$started" ] && [ -n "$cwd" ] || return 1
  [ "$(ps_field "$pid" lstart)" = "$started" ] && [ "$(cwd_of "$pid")" = "$cwd" ] || return 1
  comm="$(ps_field "$pid" comm)"
  case "$role:${comm##*/}" in serve:biorouter|install:biorouter|browser:node|serve:bash|install:bash|browser:bash) return 0 ;; *) return 1 ;; esac
}
signal_owned() {
  local role="$1" signal="$2" pid
  owns_pid "$role" || return 0
  pid="$(sed -n '1p' "$SANDBOX/$role.pid-record")"
  kill "-$signal" "$pid" 2>/dev/null || true
}
wait_for_serve() {
  local log="$1" status='000'
  for _ in $(seq 1 90); do
    [ -z "$(ps_field "$SERVE_PID" comm)" ] && fail "serve exited; see $log"
    status="$(curl -sS -m 2 -o /dev/null -w '%{http_code}' "http://127.0.0.1:$SERVE_PORT/status" 2>/dev/null || true)"
    [ "$status" != 000 ] && [ -n "$status" ] && return 0
    sleep 1
  done
  fail "serve did not answer; see $log"
}
CID=''
cleanup() {
  trap - EXIT
  for role in browser serve install; do signal_owned "$role" TERM; done
  sleep 2
  for role in browser serve install; do signal_owned "$role" KILL; done
  if [ -n "$CID" ] && [ -f "$SANDBOX/nginx.cid" ]; then
    local label
    label="$(docker inspect -f '{{ index .Config.Labels "org.biorouter.pr21-run" }}' "$CID" 2>/dev/null || true)"
    if [ "$label" = "$RUN_NAME" ]; then
      docker stop --time 10 "$CID" >/dev/null 2>&1 || true
    fi
  fi
}
trap cleanup EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

(cd "$WT" && record_pid install "$BASHPID" && exec env "${RUN_ENV[@]}" \
  bash "$HX" "$CLI" dictation install --from "$SANDBOX/model-stage" --yes \
  > "$SANDBOX/install.log" 2>&1) &
INSTALL_PID=$!
wait "$INSTALL_PID" || fail "cached --from install failed; see $SANDBOX/install.log"

TOKEN="$(openssl rand -hex 32)"
NORMAL_URL="https://127.0.0.1:$HTTPS_PORT/user/a/biorouter/"
HALVE_URL="https://127.0.0.1:$HALVE_PORT/user/a/biorouter/"

(cd "$WT" && load_versa_credential && record_pid serve "$BASHPID" && exec env "${RUN_ENV[@]}" BIOROUTER_BROWSER_TOKEN="$TOKEN" \
  "$CLI" serve --host 127.0.0.1 --port "$SERVE_PORT" --web-dir "$WEB" \
  --no-dictation-install > "$SANDBOX/serve-loopback.log" 2>&1) &
SERVE_PID=$!
wait_for_serve "$SANDBOX/serve-loopback.log"
(cd "$WT" && record_pid browser "$BASHPID" && exec env PR21_PHASE=loopback \
  PR21_RUN_ROOT="$SANDBOX" PR21_UI_DIR="$UI" PR21_WAV="$WAV" PR21_TOKEN="$TOKEN" \
  PR21_LOOPBACK_URL="http://127.0.0.1:$SERVE_PORT/" PR21_PROXY_URL="$NORMAL_URL" \
  PR21_HALVE_URL="$HALVE_URL" bash "$HX" node "$WT/scripts/check-serve-dictation-nginx-browser.mjs" \
  > "$SANDBOX/browser-loopback.log" 2>&1) &
BROWSER_PID=$!
wait "$BROWSER_PID" || fail "loopback browser check failed; see $SANDBOX/browser-loopback.log (do not treat as a skip)"
[ -f "$SANDBOX/loopback-metrics.json" ] || fail 'loopback metrics missing after browser check'
signal_owned serve TERM
for _ in $(seq 1 30); do owns_pid serve || break; sleep 0.5; done
signal_owned serve KILL
sleep 1
owns_pid serve && fail "owned loopback serve did not stop; see $SANDBOX/serve-loopback.log"

cat > "$SANDBOX/nginx.conf" <<EOF
events { worker_connections 256; }
http {
  access_log off;
  error_log /dev/stderr warn;
  server {
    listen 443 ssl;
    server_name localhost;
    ssl_certificate /tls/cert.pem;
    ssl_certificate_key /tls/key.pem;
    client_max_body_size 1m;
    location /user/a/biorouter/ {
      client_body_buffer_size 512k;
      proxy_request_buffering on;
      proxy_http_version 1.1;
      proxy_set_header Host \$http_host;
      proxy_set_header X-Forwarded-Proto https;
      proxy_set_header Upgrade \$http_upgrade;
      proxy_set_header Connection "upgrade";
      proxy_set_header Sec-Fetch-Site \$http_sec_fetch_site;
      proxy_set_header Sec-Fetch-Mode \$http_sec_fetch_mode;
      proxy_set_header Sec-Fetch-Dest \$http_sec_fetch_dest;
      proxy_pass http://host.docker.internal:$SERVE_PORT/;
    }
  }
  server {
    listen 444 ssl;
    server_name localhost;
    ssl_certificate /tls/cert.pem;
    ssl_certificate_key /tls/key.pem;
    client_max_body_size 128k;
    location /user/a/biorouter/ {
      client_body_buffer_size 512k;
      proxy_request_buffering on;
      proxy_http_version 1.1;
      proxy_set_header Host \$http_host;
      proxy_set_header X-Forwarded-Proto https;
      proxy_set_header Upgrade \$http_upgrade;
      proxy_set_header Connection "upgrade";
      proxy_set_header Sec-Fetch-Site \$http_sec_fetch_site;
      proxy_set_header Sec-Fetch-Mode \$http_sec_fetch_mode;
      proxy_set_header Sec-Fetch-Dest \$http_sec_fetch_dest;
      proxy_pass http://host.docker.internal:$SERVE_PORT/;
    }
  }
}
EOF

(cd "$WT" && load_versa_credential && record_pid serve "$BASHPID" && exec env "${RUN_ENV[@]}" BIOROUTER_BROWSER_TOKEN="$TOKEN" \
  "$CLI" serve --host 0.0.0.0 --port "$SERVE_PORT" --web-dir "$WEB" \
  --public-url "$NORMAL_URL" --no-dictation-install > "$SANDBOX/serve-proxy.log" 2>&1) &
SERVE_PID=$!
wait_for_serve "$SANDBOX/serve-proxy.log"
CID="$(docker run -d --name "br-pr21-$RUN_NAME" --label "org.biorouter.pr21-run=$RUN_NAME" \
  -p "127.0.0.1:$HTTPS_PORT:443" -p "127.0.0.1:$HALVE_PORT:444" \
  -v "$SANDBOX/nginx.conf:/etc/nginx/nginx.conf:ro" -v "$SANDBOX/tls:/tls:ro" docker.io/library/nginx:stable)" || \
  fail 'docker.io/library/nginx:stable container did not start; inspect Docker state without removing foreign containers'
printf '%s\n' "$CID" > "$SANDBOX/nginx.cid"
for _ in $(seq 1 60); do
  STATUS="$(curl -ksS -m 2 -o /dev/null -w '%{http_code}' "$NORMAL_URL" 2>/dev/null || true)"
  [ "$STATUS" != 000 ] && [ -n "$STATUS" ] && break
  sleep 1
done
[ "$STATUS" != 000 ] && [ -n "$STATUS" ] || fail "nginx HTTPS prefix did not answer; see $SANDBOX/nginx.cid"

head -c 1048578 /dev/zero > "$SANDBOX/body-1m-plus-2.bin"
STATUS="$(curl -ksS -m 30 -D "$SANDBOX/body-413.headers" \
  -o "$SANDBOX/body-413.response" -w '%{http_code}' \
  -X PUT -H 'Content-Type: application/octet-stream' -H 'Expect:' \
  --data-binary "@$SANDBOX/body-1m-plus-2.bin" \
  "${NORMAL_URL}dictation/streams/oversize-probe/audio?seq=0")"
[ "$STATUS" = 413 ] || fail "1 MiB + 2 B PUT returned $STATUS instead of 413 through normal nginx port"
docker logs "$CID" > "$SANDBOX/nginx-body-limit.log" 2>&1 || \
  fail "could not read the scoped body-limit error log for owned nginx container $CID"
if ! rg --fixed-strings 'client intended to send too large body: 1048578 bytes' \
  "$SANDBOX/nginx-body-limit.log" | \
  rg -q --fixed-strings 'PUT /user/a/biorouter/dictation/streams/oversize-probe/audio?seq=0'; then
  fail "413 lacked nginx's own 1048578-byte body-limit log for the scoped PUT; see $SANDBOX/nginx-body-limit.log"
fi
printf '%s\n' "$STATUS" > "$SANDBOX/body-413.status"

(cd "$WT" && record_pid browser "$BASHPID" && exec env PR21_PHASE=proxy PR21_RUN_ROOT="$SANDBOX" \
  PR21_UI_DIR="$UI" PR21_WAV="$WAV" PR21_TOKEN="$TOKEN" \
  PR21_LOOPBACK_URL="http://127.0.0.1:$SERVE_PORT/" PR21_PROXY_URL="$NORMAL_URL" \
  PR21_HALVE_URL="$HALVE_URL" bash "$HX" node "$WT/scripts/check-serve-dictation-nginx-browser.mjs" \
  > "$SANDBOX/browser-proxy.log" 2>&1) &
BROWSER_PID=$!
wait "$BROWSER_PID" || fail "proxy browser/halving check failed; see $SANDBOX/browser-proxy.log (do not treat as a skip)"
[ -f "$SANDBOX/browser-metrics.json" ] || fail 'browser metrics missing after browser check'
sysctl -n vm.loadavg > "$SANDBOX/load-after.txt" || fail 'could not record host load average after browser check'
docker logs "$CID" > "$SANDBOX/nginx.log" 2>&1 || fail "could not read logs for owned nginx container $CID"
if rg -q --fixed-strings 'buffered to a temporary file' "$SANDBOX/nginx.log"; then
  fail "nginx buffered audio to a temporary file; see $SANDBOX/nginx.log"
fi
printf '%s\n' 0 > "$SANDBOX/nginx-temp-buffer-lines"
printf 'PR-21 nginx evidence saved under %s\n' "$SANDBOX"
printf 'Owned nginx container %s will be stopped and retained; no image or volume will be deleted.\n' "$CID"
