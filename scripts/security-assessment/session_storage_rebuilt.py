#!/usr/bin/env python3
"""Verify rebuilt Linux daemon history through two non-root UIDs.

Requires a compatible local runtime image containing Python3 and the daemon's
shared libraries. No image is built/pulled and no host home is mounted.
"""

import argparse
import hashlib
import json
from pathlib import Path
import secrets
import subprocess
import time
import uuid


OWNER = "10001:10001"
OBSERVER = "10002:10002"
ROOT = "/home/br/app-root"
DATABASE = ROOT + "/data/sessions/sessions.db"
LABEL = "org.biorouter.assessment=rebuilt-session-storage"
READY = "import urllib.request; assert urllib.request.urlopen('http://127.0.0.1:3000/status',timeout=1).read()==b'ok'"
OWNER_PROBE = """
import json, os, sqlite3, urllib.request
from pathlib import Path
key = os.environ['BIOROUTER_SERVER__SECRET_KEY']
def api(method, path, body=None):
    req = urllib.request.Request('http://127.0.0.1:3000'+path,
        data=None if body is None else json.dumps(body).encode(), method=method,
        headers={'x-secret-key':key,'Content-Type':'application/json'})
    with urllib.request.urlopen(req,timeout=10) as response:
        return json.loads(response.read())
fixture = Path('/home/br/fixture.json')
if not fixture.exists():
    session = api('POST','/agent/start',{'working_dir':'/home/br'})
    marker = 'SYNTHETIC_REBUILT_HISTORY'
    api('PUT','/sessions/'+session['id']+'/name',{'name':marker})
    fixture.write_text(json.dumps({'id':session['id'],'marker':marker}))
saved = json.loads(fixture.read_text())
database = Path('/home/br/app-root/data/sessions/sessions.db')
with sqlite3.connect('file:'+str(database)+'?mode=ro',uri=True) as connection:
    row = connection.execute('SELECT name FROM sessions WHERE id=?',(saved['id'],)).fetchone()
assert row is not None and row[0] == saved['marker']
print(json.dumps({'owner_sqlite_read':True,'session_mode':oct(database.parent.stat().st_mode & 511)}))
"""
OBSERVER_PROBE = """
import json, os, sqlite3
from pathlib import Path
database=Path('/home/br/app-root/data/sessions/sessions.db')
try:
    with database.open('rb') as data:
        data.read(16)
except PermissionError:
    print(json.dumps({'observer_denied':True,'has_daemon_secret': 'BIOROUTER_SERVER__SECRET_KEY' in os.environ}))
else:
    with sqlite3.connect('file:'+str(database)+'?mode=ro',uri=True) as connection:
        names=[row[0] for row in connection.execute('SELECT name FROM sessions')]
    print(json.dumps({'observer_denied':False,'synthetic_marker_read':'SYNTHETIC_REBUILT_HISTORY' in names,
        'has_daemon_secret':'BIOROUTER_SERVER__SECRET_KEY' in os.environ}))
"""


def command(args, *, check=True):
    result = subprocess.run(args, capture_output=True, text=True, timeout=30)
    if check and result.returncode:
        raise RuntimeError(result.stderr or result.stdout or "subprocess failed")
    return result


def inside(container, uid, args, *, secret=None, check=True):
    cmd = ["docker", "exec", "--user", uid]
    if secret is not None:
        cmd.extend(["--env", "BIOROUTER_SERVER__SECRET_KEY=" + secret])
    return command([*cmd, container, *args], check=check)


def start_daemon(container, secret):
    inside(container, OWNER, [
        "sh", "-c", "umask 022; /probe/biorouterd agent </dev/null >/home/br/daemon.stdout "
        "2>/home/br/daemon.stderr & echo $! >/home/br/daemon.pid",
    ], secret=secret)
    for _ in range(60):
        if inside(container, OWNER, ["python3", "-c", READY], check=False).returncode == 0:
            return
        time.sleep(0.2)
    error = inside(container, OWNER, ["cat", "/home/br/daemon.stderr"], check=False)
    raise RuntimeError("rebuilt daemon did not start: " + error.stdout)


def stop_daemon(container):
    inside(container, OWNER, ["sh", "-c", "kill -TERM $(cat /home/br/daemon.pid)"])
    for _ in range(60):
        if inside(container, OWNER, ["sh", "-c", "kill -0 $(cat /home/br/daemon.pid)"], check=False).returncode != 0:
            return
        time.sleep(0.2)
    raise RuntimeError("rebuilt daemon did not stop")


def main(args):
    daemon = args.daemon.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    image = json.loads(command(["docker", "image", "inspect", args.image]).stdout)[0]
    results = []
    for repetition in range(1, args.repetitions + 1):
        container = "brsec-rebuilt-history-" + uuid.uuid4().hex[:12]
        secret = secrets.token_hex(32)
        command([
            "docker", "run", "-d", "--init", "--name", container, "--label", LABEL,
            "--network", "none", "--read-only", "--cap-drop", "ALL",
            "--security-opt", "no-new-privileges", "--pids-limit", "128", "--cpus", "1",
            "--memory", "2g", "--memory-swap", "2g", "--user", OWNER,
            "--tmpfs", "/tmp:rw,nosuid,nodev,size=64m,mode=1777",
            "--tmpfs", "/home/br:rw,nosuid,nodev,size=256m,mode=755,uid=10001,gid=10001",
            "--mount", "type=bind,src=" + str(daemon) + ",dst=/probe/biorouterd,readonly",
            "--env", "HOME=/home/br", "--env", "BIOROUTER_PATH_ROOT=" + ROOT,
            "--env", "BIOROUTER_HOST=127.0.0.1", "--env", "BIOROUTER_PORT=3000",
            "--env", "BIOROUTER_DISABLE_KEYRING=true", "--workdir", "/home/br",
            image["Id"], "sleep", "600",
        ])
        try:
            start_daemon(container, secret)
            owner_fresh = json.loads(inside(container, OWNER, ["python3", "-c", OWNER_PROBE], secret=secret).stdout)
            inside(container, OWNER, ["chmod", "755", ROOT, ROOT + "/data"])
            outsider_fresh = json.loads(inside(container, OBSERVER, ["python3", "-c", OBSERVER_PROBE]).stdout)
            assert owner_fresh["owner_sqlite_read"] and owner_fresh["session_mode"] == "0o700"
            assert outsider_fresh["observer_denied"] and not outsider_fresh["has_daemon_secret"]
            stop_daemon(container)
            inside(container, OWNER, ["chmod", "755", ROOT + "/data/sessions"])
            outsider_legacy = json.loads(inside(container, OBSERVER, ["python3", "-c", OBSERVER_PROBE]).stdout)
            assert outsider_legacy.get("synthetic_marker_read"), "legacy fixture must demonstrate readability before repair"
            start_daemon(container, secret)
            owner_reopened = json.loads(inside(container, OWNER, ["python3", "-c", OWNER_PROBE], secret=secret).stdout)
            outsider_repaired = json.loads(inside(container, OBSERVER, ["python3", "-c", OBSERVER_PROBE]).stdout)
            assert owner_reopened["owner_sqlite_read"] and owner_reopened["session_mode"] == "0o700"
            assert outsider_repaired["observer_denied"] and not outsider_repaired["has_daemon_secret"]
            result = {"repetition": repetition, "owner_fresh": owner_fresh, "outsider_fresh": outsider_fresh,
                      "outsider_legacy_before_restart": outsider_legacy,
                      "owner_reopened": owner_reopened, "outsider_repaired": outsider_repaired}
            results.append(result)
            (output / f"repetition-{repetition}.json").write_text(json.dumps(result, indent=2) + "\n")
            (output / f"repetition-{repetition}-daemon.stderr").write_text(
                inside(container, OWNER, ["cat", "/home/br/daemon.stderr"]).stdout)
            stop_daemon(container)
        finally:
            label = command(["docker", "inspect", "--format", '{{ index .Config.Labels "org.biorouter.assessment" }}', container], check=False)
            if label.stdout.strip() == "rebuilt-session-storage":
                command(["docker", "rm", "-f", container], check=False)
    summary = {"daemon_sha256": hashlib.sha256(daemon.read_bytes()).hexdigest(),
               "declared_source_commit": args.source_commit, "source_commit_is_operator_supplied": True,
               "runtime_image_id": image["Id"], "architecture": image["Architecture"],
               "owner_uid": 10001, "observer_uid": 10002, "results": results,
               "limitation": "Synthetic rebuilt-daemon/SQLite access and restart tests; not same-user or administrator isolation."}
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--daemon", type=Path, required=True)
    parser.add_argument("--image", required=True, help="Existing compatible local runtime image")
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--output", type=Path, required=True, help="New directory; never overwrite frozen evidence")
    parser.add_argument("--repetitions", type=int, choices=range(1, 4), default=3)
    main(parser.parse_args())
