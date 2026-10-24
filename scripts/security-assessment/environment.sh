#!/usr/bin/env bash
set -euo pipefail

ASSESSMENT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
RUNTIME="${BRSEC_RUNTIME:-/Users/wgu/biorouter-runs/security-assessment-2026-10}"
IMAGE="brsec-runtime:1.93.4-amd64"
CONTAINER="brsec-target"
PACKAGE="biorouter-cli_1.93.4_amd64.deb"
PACKAGE_SHA="8b689a2006f1b99f44dec9a48a03b8bb67759dd7f11cdc288042a8a985abff65"
SECRET="ba11ab1e00000000000000000000000000000000000000000000000000000001"
BROWSER_TOKEN="ba11ab1e00000000000000000000000000000000000000000000000000000002"

case "${1:-}" in
  prepare)
    mkdir -p "$RUNTIME/fixtures" "$RUNTIME/evidence/environment"
    if [[ ! -f "$RUNTIME/$PACKAGE" ]]; then
      gh release download v1.93.4 --repo BaranziniLab/biorouter --pattern "$PACKAGE" --dir "$RUNTIME"
    fi
    [[ "$(shasum -a 256 "$RUNTIME/$PACKAGE" | cut -d ' ' -f 1)" == "$PACKAGE_SHA" ]]
    cp "$ASSESSMENT_ROOT/scripts/security-assessment/environment.Dockerfile" "$RUNTIME/Dockerfile"
    docker build --platform linux/amd64 --tag "$IMAGE" "$RUNTIME"
    docker image inspect "$IMAGE" > "$RUNTIME/evidence/environment/image-inspect.json"
    git -C "$ASSESSMENT_ROOT" diff b824e2c3e802e243804a150aa7c349fa6ce6a8b6 f73ebf74c578717a7af64b221e57358a63dda8e3 > "$RUNTIME/evidence/environment/release-to-assessment.diff"
    ;;
  start)
    REPETITION="${2:?usage: environment.sh start repetition}"
    [[ "$REPETITION" =~ ^[1-3]$ ]]
    if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
      echo "$CONTAINER already exists; run environment.sh stop first" >&2
      exit 1
    fi
    mkdir -p "$RUNTIME/fixtures" "$RUNTIME/evidence/environment" "$RUNTIME/evidence/network/repetition-$REPETITION" "$RUNTIME/evidence/local/repetition-$REPETITION"
    chmod 777 "$RUNTIME/evidence/network/repetition-$REPETITION" "$RUNTIME/evidence/local/repetition-$REPETITION"
    docker run -d --name "$CONTAINER" --label org.biorouter.assessment=security-2026-10 \
      --platform linux/amd64 --network none --read-only --cap-drop ALL \
      --security-opt no-new-privileges --pids-limit 128 --cpus 1 --memory 2g --memory-swap 2g \
      --user 10001:10001 \
      --tmpfs /tmp:rw,nosuid,nodev,size=128m,mode=1777 \
      --tmpfs /home/br:rw,nosuid,nodev,size=256m,mode=755,uid=10001,gid=10001 \
      --tmpfs /home/attacker:rw,nosuid,nodev,size=16m,mode=700,uid=10002,gid=10002 \
      --mount "type=bind,src=$RUNTIME/fixtures,dst=/assessment/fixtures,readonly" \
      --mount "type=bind,src=$ASSESSMENT_ROOT/scripts/security-assessment,dst=/assessment/harness,readonly" \
      --mount "type=bind,src=$RUNTIME/evidence,dst=/evidence" \
      --env HOME=/home/br --env XDG_CONFIG_HOME=/home/br/.config --env BIOROUTER_PATH_ROOT=/home/br/state \
      --env BIOROUTER_HOST=127.0.0.1 --env BIOROUTER_PORT=3000 \
      --env BIOROUTER_DISABLE_KEYRING=true --env BIOROUTER_SERVE_UI=/opt/release/usr/share/biorouter/web \
      "$IMAGE"
    docker inspect "$CONTAINER" > "$RUNTIME/evidence/environment/container-$REPETITION.json"
    docker exec --user 10001:10001 --env "BIOROUTER_SERVER__SECRET_KEY=$SECRET" --env "BIOROUTER_BROWSER_TOKEN=$BROWSER_TOKEN" -d "$CONTAINER" sh -c 'umask 022; biorouterd agent </dev/null >/home/br/daemon.stdout 2>/home/br/daemon.stderr'
    docker exec -i --user 10002:10002 "$CONTAINER" python3 - <<'PY'
import time
import urllib.error
import urllib.request
for attempt in range(30):
    try:
        assert urllib.request.urlopen("http://127.0.0.1:3000/status", timeout=1).read() == b"ok"
        break
    except (OSError, urllib.error.URLError):
        time.sleep(0.2)
else:
    raise RuntimeError("Daemon did not become ready")
PY
    echo "Container $CONTAINER started for repetition $REPETITION. Daemon loopback: http://127.0.0.1:3000"
    ;;
  inspect)
    docker exec --user 10001:10001 "$CONTAINER" python3 -c 'import json,os,platform,subprocess; from pathlib import Path; print(json.dumps({"uid":os.getuid(),"gid":os.getgid(),"uname":platform.uname()._asdict(),"python":platform.python_version(),"status":{x.split(":",1)[0]:x.split(":",1)[1].strip() for x in Path("/proc/self/status").read_text().splitlines() if x.startswith(("Cap","NoNewPrivs","Seccomp"))},"mountinfo":Path("/proc/self/mountinfo").read_text(),"binary_sha256":subprocess.check_output(["sha256sum","/opt/biorouter/biorouter","/opt/biorouter/biorouterd"],text=True),"version":subprocess.run(["biorouterd","--version"],capture_output=True,text=True).stdout,"network_interfaces":sorted(x.name for x in Path("/sys/class/net").iterdir())},indent=2))'
    ;;
  stop)
    if docker container inspect "$CONTAINER" >/dev/null 2>&1; then
      [[ "$(docker inspect --format '{{ index .Config.Labels "org.biorouter.assessment" }}' "$CONTAINER")" == security-2026-10 ]]
      docker rm -f "$CONTAINER"
    fi
    ;;
  *)
    echo 'usage: environment.sh {prepare|start <1..3>|inspect|stop}' >&2
    exit 2
    ;;
esac
