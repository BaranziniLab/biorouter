#!/usr/bin/env python3
"""Verify rebuilt Linux daemon history through two non-root UIDs.

Requires a compatible local runtime image containing Python3 and the daemon's
shared libraries. No image is built/pulled and no host home is mounted.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time
import uuid


OWNER = "10001:10001"
OBSERVER = "10002:10002"
ROOT = "/home/br/app-root"
LABEL = "org.biorouter.assessment=rebuilt-session-storage"
CASE_IDS = ["container_isolation", "owner_public_control", "observer_public_control",
            "runtime_compatibility", "owner_fresh_history", "observer_fresh_history",
            "observer_legacy_readable_control", "owner_persisted_history", "observer_repaired_history"]
READY = "import urllib.request; assert urllib.request.urlopen('http://127.0.0.1:3000/status',timeout=1).read()==b'ok'"
OWNER_PROBE = """
import json, os, sqlite3, urllib.request
from pathlib import Path
key = os.environ['BIOROUTER_SERVER__SECRET_KEY']
def api(method, path, body=None, *, expect_json=True):
    req = urllib.request.Request('http://127.0.0.1:3000'+path,
        data=None if body is None else json.dumps(body).encode(), method=method,
        headers={'x-secret-key':key,'Content-Type':'application/json'})
    with urllib.request.urlopen(req,timeout=10) as response:
        data = response.read()
        if expect_json:
            return json.loads(data)
fixture = Path('/home/br/fixture.json')
if not fixture.exists():
    session = api('POST','/agent/start',{'working_dir':'/home/br'})
    marker = 'SYNTHETIC_REBUILT_HISTORY'
    api('PUT','/sessions/'+session['id']+'/name',{'name':marker},expect_json=False)
    fixture.write_text(json.dumps({'id':session['id'],'marker':marker}))
saved = json.loads(fixture.read_text())
database = Path('/home/br/app-root/data/sessions/sessions.db')
with sqlite3.connect('file:'+str(database)+'?mode=ro',uri=True) as connection:
    row = connection.execute('SELECT name FROM sessions WHERE id=?',(saved['id'],)).fetchone()
assert row is not None and row[0] == saved['marker']
assert api('GET','/sessions/'+saved['id']+'?metadata_only=true')['name'] == saved['marker']
daemon_pid = Path('/home/br/daemon.pid').read_text().strip()
daemon_status = dict(line.split(':',1) for line in Path('/proc/'+daemon_pid+'/status').read_text().splitlines() if ':' in line)
daemon_euid = int(daemon_status['Uid'].split()[1])
assert os.geteuid() == 10001 and daemon_euid == 10001
print(json.dumps({'owner_euid':os.geteuid(),'daemon_euid':daemon_euid,'owner_sqlite_read':True,
    'owner_api_read':True,'session_mode':oct(database.parent.stat().st_mode & 511),
    'database_mode':oct(database.stat().st_mode & 511)}))
"""
OBSERVER_PROBE = """
import json, os, sqlite3
from pathlib import Path
database=Path('/home/br/app-root/data/sessions/sessions.db')
result={'observer_euid':os.geteuid(),'raw_file_denied':False,'sqlite_denied':False,
    'synthetic_marker_read':False,'has_daemon_secret':'BIOROUTER_SERVER__SECRET_KEY' in os.environ}
assert os.geteuid() == 10002
try:
    with database.open('rb') as data:
        data.read(16)
except PermissionError:
    result['raw_file_denied']=True
try:
    with sqlite3.connect('file:'+str(database)+'?mode=ro',uri=True) as connection:
        names=[row[0] for row in connection.execute('SELECT name FROM sessions')]
    result['synthetic_marker_read']='SYNTHETIC_REBUILT_HISTORY' in names
except sqlite3.Error as error:
    result['sqlite_denied']=True
    result['sqlite_error_class']=type(error).__name__
result['observer_denied']=result['raw_file_denied'] and result['sqlite_denied']
print(json.dumps(result))
"""
ISOLATION_PROBE = """
import json, os
from pathlib import Path
status=dict(line.split(':',1) for line in Path('/proc/self/status').read_text().splitlines() if ':' in line)
defaults=[line for line in Path('/proc/net/route').read_text().splitlines()[1:] if line.split()[1]=='00000000']
assert Path('/home/br/public-control.txt').read_text() == 'SYNTHETIC_PUBLIC_CONTROL'
print(json.dumps({'euid':os.geteuid(),'egid':os.getegid(),'public_control_read':True,
    'cap_eff':status['CapEff'].strip(),'no_new_privs':status['NoNewPrivs'].strip(),
    'seccomp':status['Seccomp'].strip(),'default_routes':len(defaults),
    'has_daemon_secret':'BIOROUTER_SERVER__SECRET_KEY' in os.environ}))
"""


def command(args, *, check=True, env=None):
    try:
        result = subprocess.run(args, capture_output=True, text=True, timeout=30, env=env)
    except subprocess.TimeoutExpired:
        raise RuntimeError("fixture subprocess exceeded 30 seconds") from None
    if check and result.returncode:
        raise RuntimeError(result.stderr or result.stdout or "subprocess failed")
    return result


def inside(container, uid, args, *, secret=None, check=True):
    cmd = ["docker", "exec", "--user", uid]
    env = None
    if secret is not None:
        cmd.extend(["--env", "BIOROUTER_SERVER__SECRET_KEY"])
        env = dict(os.environ, BIOROUTER_SERVER__SECRET_KEY=secret)
    return command([*cmd, container, *args], check=check, env=env)


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
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        if inside(container, OWNER, ["sh", "-c", "kill -0 $(cat /home/br/daemon.pid)"], check=False).returncode != 0:
            return
        time.sleep(0.2)
    raise RuntimeError("rebuilt daemon did not stop")


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2) + "\n")


def record_case(result, case_id, observed, assertions):
    passed = all(assertions.values())
    result["cases"].append({"case_id": case_id, "observed": observed,
                            "assertions": assertions, "passed": passed})
    if not passed:
        raise RuntimeError("fixture assertions failed: " + case_id)


def capture_daemon_logs(output, prefix, container, secret):
    for stream in ("stdout", "stderr"):
        log = inside(container, OWNER, ["cat", "/home/br/daemon." + stream], check=False)
        (output / f"{prefix}-daemon.{stream}").write_text(
            (log.stdout + log.stderr).replace(secret, "<synthetic-daemon-secret>"))


def main(args):
    daemon = args.daemon.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    results = []
    summary = {"status": "running", "daemon_sha256": hashlib.sha256(daemon.read_bytes()).hexdigest(),
               "declared_source_commit": args.source_commit, "source_commit_is_operator_supplied": True,
               "requested_repetitions": args.repetitions, "owner_uid": 10001, "observer_uid": 10002,
               "expected_case_ids_per_repetition": CASE_IDS, "results": results,
               "limitation": "Synthetic rebuilt-daemon/SQLite access and restart tests; not same-user or administrator isolation, a release build, or a HIPAA assessment."}
    write_json(output / "summary.json", summary)
    try:
        image = json.loads(command(["docker", "image", "inspect", args.image]).stdout)[0]
        summary.update(runtime_image_id=image["Id"], architecture=image["Architecture"])
    except Exception as error:
        summary.update(status="failed", setup_error=str(error))
        write_json(output / "summary.json", summary)
        return 1
    for repetition in range(1, args.repetitions + 1):
        container = "brsec-rebuilt-history-" + uuid.uuid4().hex[:12]
        secret = secrets.token_hex(32)
        result = {"repetition": repetition, "status": "running", "phase": "container_start", "cases": []}
        results.append(result)
        try:
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
                "--env", "BIOROUTER_DISABLE_KEYRING=true", "--env", "TOKIO_WORKER_THREADS=2",
                "--workdir", "/home/br", image["Id"], "sleep", "600",
            ])
            result["phase"] = "isolation_controls"
            inspected = json.loads(command(["docker", "inspect", container]).stdout)[0]
            host = inspected["HostConfig"]
            mounts = [{"type": m["Type"], "destination": m["Destination"], "writable": m["RW"]}
                      for m in inspected["Mounts"]]
            record_case(result, "container_isolation", {"network_mode": host["NetworkMode"],
                        "read_only": host["ReadonlyRootfs"], "user": inspected["Config"]["User"],
                        "cap_drop": host["CapDrop"], "security_opt": host["SecurityOpt"], "mounts": mounts},
                        {"network_none": host["NetworkMode"] == "none",
                         "root_readonly": host["ReadonlyRootfs"], "nonroot_owner": inspected["Config"]["User"] == OWNER,
                         "all_caps_dropped": "ALL" in host["CapDrop"],
                         "no_new_privileges": any(x.startswith("no-new-privileges") for x in host["SecurityOpt"]),
                         "only_daemon_host_mount": [m for m in mounts if m["type"] == "bind"] == [
                             {"type": "bind", "destination": "/probe/biorouterd", "writable": False}],
                         "no_published_ports": not host["PortBindings"]})
            inside(container, OWNER, ["python3", "-c", "from pathlib import Path; p=Path('/home/br/public-control.txt'); p.write_text('SYNTHETIC_PUBLIC_CONTROL'); p.chmod(0o644)"])
            for actor, uid in (("owner", OWNER), ("observer", OBSERVER)):
                observed = json.loads(inside(container, uid, ["python3", "-c", ISOLATION_PROBE]).stdout)
                record_case(result, actor + "_public_control", observed,
                            {"expected_euid": observed["euid"] == int(uid.split(":")[0]),
                             "public_canary_read": observed["public_control_read"],
                             "no_secret": not observed["has_daemon_secret"], "zero_capabilities": int(observed["cap_eff"], 16) == 0,
                             "no_new_privileges": observed["no_new_privs"] == "1", "seccomp": observed["seccomp"] == "2",
                             "no_default_route": observed["default_routes"] == 0})
            compatibility = inside(container, OWNER, ["sh", "-ec", "getconf GNU_LIBC_VERSION; ldd /probe/biorouterd; /probe/biorouterd --version"], check=False)
            (output / f"repetition-{repetition}-runtime.txt").write_text(compatibility.stdout + compatibility.stderr)
            record_case(result, "runtime_compatibility", {"exit_code": compatibility.returncode},
                        {"daemon_loads": compatibility.returncode == 0, "no_missing_library": "not found" not in compatibility.stdout})
            result["phase"] = "fresh_history"
            start_daemon(container, secret)
            owner_fresh = json.loads(inside(container, OWNER, ["python3", "-c", OWNER_PROBE], secret=secret).stdout)
            inside(container, OWNER, ["chmod", "755", ROOT, ROOT + "/data"])
            outsider_fresh = json.loads(inside(container, OBSERVER, ["python3", "-c", OBSERVER_PROBE]).stdout)
            record_case(result, "owner_fresh_history", owner_fresh,
                        {"sqlite_title_read": owner_fresh["owner_sqlite_read"], "api_title_read": owner_fresh["owner_api_read"],
                         "daemon_euid": owner_fresh["daemon_euid"] == 10001, "owner_euid": owner_fresh["owner_euid"] == 10001,
                         "directory_0700": owner_fresh["session_mode"] == "0o700", "database_0600": owner_fresh["database_mode"] == "0o600"})
            record_case(result, "observer_fresh_history", outsider_fresh,
                        {"raw_open_denied": outsider_fresh["raw_file_denied"], "sqlite_denied": outsider_fresh["sqlite_denied"],
                         "marker_not_read": not outsider_fresh["synthetic_marker_read"], "no_secret": not outsider_fresh["has_daemon_secret"]})
            capture_daemon_logs(output, f"repetition-{repetition}-initial", container, secret)
            stop_daemon(container)
            result["phase"] = "legacy_positive_control"
            inside(container, OWNER, ["chmod", "755", ROOT + "/data/sessions"])
            inside(container, OWNER, ["python3", "-c", "import sqlite3; from pathlib import Path; p=Path('/home/br/app-root/data/sessions/sessions.db'); c=sqlite3.connect(p); c.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchall(); c.close(); [f.chmod(0o644) for f in p.parent.iterdir() if f.name in ('sessions.db','sessions.db-wal','sessions.db-shm')]"])
            outsider_legacy = json.loads(inside(container, OBSERVER, ["python3", "-c", OBSERVER_PROBE]).stdout)
            record_case(result, "observer_legacy_readable_control", outsider_legacy,
                        {"raw_open_allowed": not outsider_legacy["raw_file_denied"], "sqlite_allowed": not outsider_legacy["sqlite_denied"],
                         "synthetic_title_read": outsider_legacy["synthetic_marker_read"], "no_secret": not outsider_legacy["has_daemon_secret"]})
            result["phase"] = "restart_and_repair"
            start_daemon(container, secret)
            owner_reopened = json.loads(inside(container, OWNER, ["python3", "-c", OWNER_PROBE], secret=secret).stdout)
            outsider_repaired = json.loads(inside(container, OBSERVER, ["python3", "-c", OBSERVER_PROBE]).stdout)
            record_case(result, "owner_persisted_history", owner_reopened,
                        {"same_sqlite_title_read": owner_reopened["owner_sqlite_read"], "same_api_title_read": owner_reopened["owner_api_read"],
                         "daemon_euid": owner_reopened["daemon_euid"] == 10001, "owner_euid": owner_reopened["owner_euid"] == 10001,
                         "directory_repaired_0700": owner_reopened["session_mode"] == "0o700", "database_repaired_0600": owner_reopened["database_mode"] == "0o600"})
            record_case(result, "observer_repaired_history", outsider_repaired,
                        {"raw_open_denied": outsider_repaired["raw_file_denied"], "sqlite_denied": outsider_repaired["sqlite_denied"],
                         "marker_not_read": not outsider_repaired["synthetic_marker_read"], "no_secret": not outsider_repaired["has_daemon_secret"]})
            stop_daemon(container)
            result.update(status="passed", phase="completed")
        except Exception as error:
            result.update(status="failed", error_class=type(error).__name__, error=str(error).replace(secret, "<synthetic-daemon-secret>"))
        finally:
            try:
                label = command(["docker", "inspect", "--format", '{{ index .Config.Labels "org.biorouter.assessment" }}', container], check=False)
                if label.returncode or label.stdout.strip() != "rebuilt-session-storage":
                    raise RuntimeError("container identity could not be verified; deletion was not attempted")
                try:
                    capture_daemon_logs(output, f"repetition-{repetition}-final", container, secret)
                finally:
                    cleanup = command(["docker", "rm", "-f", container], check=False)
                    if cleanup.returncode:
                        raise RuntimeError("test-owned container cleanup failed")
                    result["cleanup"] = {"label_verified": True, "removed": True}
            except Exception as error:
                result.update(status="failed", cleanup_error=str(error).replace(secret, "<synthetic-daemon-secret>"))
            observed_cases = {case["case_id"] for case in result["cases"]}
            result["not_exercised_case_ids"] = [case for case in CASE_IDS if case not in observed_cases]
            write_json(output / f"repetition-{repetition}.json", result)
            write_json(output / "summary.json", summary)
    summary["status"] = "passed" if all(row["status"] == "passed" for row in results) else "failed"
    write_json(output / "summary.json", summary)
    print(json.dumps(summary, indent=2))
    return 0 if summary["status"] == "passed" else 1


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--daemon", type=Path, required=True)
    parser.add_argument("--image", required=True, help="Existing compatible local runtime image")
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--output", type=Path, required=True, help="New directory; never overwrite frozen evidence")
    parser.add_argument("--repetitions", type=int, choices=range(1, 4), default=3)
    arguments = parser.parse_args()
    if not re.fullmatch(r"[0-9a-f]{40}", arguments.source_commit):
        parser.error("--source-commit must be an exact lowercase 40-character source SHA")
    sys.exit(main(arguments))
