#!/usr/bin/env bash
# Check the shipped Docker image with local fixtures and a pinned model cache.
set -euo pipefail
set +x

usage() {
  echo 'usage: check-docker-dictation.sh --image TAG --model-cache DIR --run-root DIR [--browser]' >&2
  exit 2
}

IMAGE=''
MODEL_CACHE=''
RUN_ROOT=''
BROWSER=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --image) [ "$#" -ge 2 ] || usage; IMAGE="$2"; shift 2 ;;
    --model-cache) [ "$#" -ge 2 ] || usage; MODEL_CACHE="$2"; shift 2 ;;
    --run-root) [ "$#" -ge 2 ] || usage; RUN_ROOT="$2"; shift 2 ;;
    --browser) BROWSER=1; shift ;;
    *) usage ;;
  esac
done
[ -n "$IMAGE" ] && [ -n "$MODEL_CACHE" ] && [ -n "$RUN_ROOT" ] || usage
[ ! -e "$RUN_ROOT" ] || { echo "run root exists: $RUN_ROOT" >&2; exit 2; }
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MODEL_CACHE="$(cd "$MODEL_CACHE" && pwd -P)"
RUN_ROOT="$(mkdir -p "$RUN_ROOT" && cd "$RUN_ROOT" && pwd -P)"
FIXTURES="$ROOT/vendor/whisper.cpp/source"
MODEL_SHA=67a179f608ea6114bd3fdb9060e762b588a3fb3bd00c4387971be4d177958067
VAD_SHA=2aa269b785eeb53a82983a20501ddf7c1d9c48e33ab63a41391ac6c9f7fb6987
python3 - "$ROOT/crates/biorouter-dictation/src/catalog.rs" "$MODEL_SHA" "$VAD_SHA" <<'PY'
import re, sys
catalog = open(sys.argv[1]).read()
for name, expected in zip(('SMALL_EN_SHA256', 'VAD_SHA256'), sys.argv[2:]):
    match = re.search(rf'const {name}: &str = "([0-9a-f]{{64}})";', catalog)
    assert match and match.group(1) == expected, f'{name} changed; update the Docker cache pin'
PY
printf '%s  %s\n' "$MODEL_SHA" "$MODEL_CACHE/ggml-small.en-q8_0.bin" \
  "$VAD_SHA" "$MODEL_CACHE/ggml-silero-v6.2.0.bin" | shasum -a 256 -c -
command -v docker >/dev/null
docker image inspect "$IMAGE" >/dev/null

docker run --rm "$IMAGE" --version | tee "$RUN_ROOT/version.txt"
for binary in biorouter biorouterd; do
  docker run --rm --mount "type=bind,src=$FIXTURES,dst=/fixtures,readonly" \
    --entrypoint "/usr/local/bin/$binary" "$IMAGE" \
    __br-dictation-worker self-test \
    --model /fixtures/models/for-tests-ggml-base.en.bin \
    --vad /fixtures/models/for-tests-silero-v6.2.0-ggml.bin \
    --wav /fixtures/samples/jfk.wav | tee "$RUN_ROOT/$binary-self-test.json"
  python3 - "$RUN_ROOT/$binary-self-test.json" <<'PY'
import json, sys
rows = open(sys.argv[1]).read().splitlines()
assert len(rows) == 1 and json.loads(rows[0])["selftest"] == "ok"
PY
done

docker run --rm "$IMAGE" dictation status --format json > "$RUN_ROOT/no-model.json"
python3 - "$RUN_ROOT/no-model.json" <<'PY'
import json, sys
status = json.load(open(sys.argv[1]))
assert status["active_model"] is None
assert not any(row["installed"] for row in status["models"])
PY
docker run --rm -i --entrypoint python3 "$IMAGE" - <<'PY'
import os
for root in ('/usr/local', '/usr/share', '/home/biorouter'):
    for folder, _, names in os.walk(root):
        for name in names:
            path = os.path.join(folder, name)
            if os.path.isfile(path):
                with open(path, 'rb') as stream:
                    assert stream.read(4) != b'lmgg', f'model bytes inside image: {path}'
PY

RUN_ID="$(printf '%s' "$(basename "$RUN_ROOT")" | tr -c 'a-zA-Z0-9' '-')"
DATA_VOLUME="br-pr31-data-${RUN_ID}"
CONFIG_VOLUME="br-pr31-config-${RUN_ID}"
CONTAINER="br-pr31-serve-${RUN_ID}"
for volume in "$DATA_VOLUME" "$CONFIG_VOLUME"; do
  if docker volume inspect "$volume" >/dev/null 2>&1; then
    echo "volume already exists, choose a fresh run root: $volume" >&2
    exit 2
  fi
done
if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
  echo "container already exists, choose a fresh run root: $CONTAINER" >&2
  exit 2
fi
docker volume create "$DATA_VOLUME" > "$RUN_ROOT/data-volume.txt"
docker volume create "$CONFIG_VOLUME" > "$RUN_ROOT/config-volume.txt"
docker run --rm --mount "type=volume,src=$DATA_VOLUME,dst=/home/biorouter/.local/share/biorouter" \
  --mount "type=volume,src=$CONFIG_VOLUME,dst=/home/biorouter/.config/biorouter" \
  --mount "type=bind,src=$MODEL_CACHE,dst=/model-cache,readonly" \
  "$IMAGE" dictation install --yes --from /model-cache > "$RUN_ROOT/install.txt"
docker run --rm --mount "type=volume,src=$DATA_VOLUME,dst=/home/biorouter/.local/share/biorouter" \
  --mount "type=volume,src=$CONFIG_VOLUME,dst=/home/biorouter/.config/biorouter" \
  --mount "type=bind,src=$FIXTURES/samples,dst=/samples,readonly" \
  "$IMAGE" transcribe /samples/jfk.wav --format json > "$RUN_ROOT/transcribe.json"
python3 - "$RUN_ROOT/transcribe.json" <<'PY'
import json, re, sys
result = json.load(open(sys.argv[1]))
words = ' '.join(re.findall(r'[a-z0-9]+', result['text'].lower()))
fragments = (
    'my fellow americans',
    'ask not what your country can do for you',
    'ask what you can do for your country',
)
assert all(fragment in words for fragment in fragments), 'JFK sentence was incomplete'
assert result['bias_scope'] == 'public'
PY

docker run --rm -d --name "$CONTAINER" -p 127.0.0.1:8765:8765 \
  -e BIOROUTER_PROVIDER=versa_azure -e BIOROUTER_MODEL=gpt-5.5-2026-04-24 \
  -e VERSA_AZURE_API_KEY=pr31-unused-test-key \
  --mount "type=volume,src=$DATA_VOLUME,dst=/home/biorouter/.local/share/biorouter" \
  --mount "type=volume,src=$CONFIG_VOLUME,dst=/home/biorouter/.config/biorouter" \
  "$IMAGE" serve --host 0.0.0.0 --token pr31-local-check > "$RUN_ROOT/container-id.txt"
stop_serve() { docker stop "$CONTAINER" >/dev/null 2>&1 || true; }
trap stop_serve EXIT
# The token exchange answers with a session cookie and a page that bounces to the
# shell (routes/web_ui.rs exchange_bounce), so keep the cookie and fetch the shell
# with it, the way a browser follows the bounce.
for _ in $(seq 1 90); do
  if curl -fsS -c "$RUN_ROOT/cookies.txt" -o "$RUN_ROOT/exchange.html" \
    'http://127.0.0.1:8765/?t=pr31-local-check'; then break; fi
  sleep 1
done
test -s "$RUN_ROOT/exchange.html"
grep -q 'Opening Biorouter' "$RUN_ROOT/exchange.html"
curl -fsS -b "$RUN_ROOT/cookies.txt" -o "$RUN_ROOT/shell.html" 'http://127.0.0.1:8765/'
test -s "$RUN_ROOT/shell.html"
grep -q '__BIOROUTER_HEADLESS_CONFIG__' "$RUN_ROOT/shell.html"
docker logs "$CONTAINER" > "$RUN_ROOT/serve.log" 2>&1
grep -q 'This runs in a container. On the computer that publishes its port' "$RUN_ROOT/serve.log"
if [ "$BROWSER" -eq 1 ]; then
  PR31_UI_DIR="$ROOT/ui/desktop" \
    PR31_URL='http://127.0.0.1:8765/' \
    PR31_WAV="$FIXTURES/samples/jfk.wav" \
    PR31_RUN_ROOT="$RUN_ROOT" \
    node "$ROOT/scripts/check-docker-dictation-browser.mjs"
fi
echo "PR-31 image checks passed; evidence in $RUN_ROOT; volumes $DATA_VOLUME and $CONFIG_VOLUME retained"
