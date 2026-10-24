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
            "owner_fresh_undo", "observer_fresh_undo", "observer_legacy_readable_control",
            "owner_persisted_history", "observer_repaired_history", "owner_persisted_undo",
            "observer_reopened_undo"]
READY = "import urllib.request; assert urllib.request.urlopen('http://127.0.0.1:3000/status',timeout=1).read()==b'ok'"
API_PROBE = """
import json, os, urllib.request
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
"""
OWNER_PROBE = API_PROBE + """
import sqlite3
fixture = Path('/home/br/fixture.json')
if not fixture.exists():
    session = api('POST','/agent/start',{'working_dir':'/home/br','extension_overrides':[
        {'type':'builtin','name':'developer','description':'','display_name':None,
         'timeout':30,'bundled':True,'available_tools':[]}]})
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
UNDO_PROBE = API_PROBE + """
import sys
assert os.geteuid() == 10001
phase = sys.argv[1]
saved = json.loads(Path('/home/br/fixture.json').read_text())
target = Path('/home/br/undo-target.txt')
undo_directory = Path('/home/br/app-root/config/developer_undo')
original = 'SYNTHETIC_PRIVATE_UNDO_ORIGINAL\\n'
edited = 'SYNTHETIC_UNDO_EDITED\\n'
def edit(command, **arguments):
    result = api('POST','/agent/call_tool',{'session_id':saved['id'],
        'name':'developer__text_editor','arguments':{'command':command,'path':str(target),**arguments}})
    assert result.get('is_error') is False, 'Developer editor did not complete successfully'
    return result
def snapshot_count(history):
    return sum((snapshot if isinstance(snapshot,str) else snapshot['content']) == original
        for snapshots in history['files'].values() for snapshot in snapshots)
if phase == 'write':
    target.write_text(original)
    edit('write',file_text=edited)
    assert target.read_text() == edited
    stores = list(undo_directory.glob('*.json'))
    assert len(stores) == 1, 'Expected one real workspace undo store'
    store = stores[0]
    history = json.loads(store.read_text())
    assert snapshot_count(history) == 1, 'Original file was not persisted by Developer'
    saved['undo_store'] = str(store)
    Path('/home/br/fixture.json').write_text(json.dumps(saved))
    outcome = {'editor_write_succeeded':True,'edited_file_matches':True,
        'persisted_original_snapshot':True,'original_snapshot_count':snapshot_count(history)}
elif phase == 'restore':
    resumed = api('POST','/agent/resume',{'session_id':saved['id'],'load_model_and_extensions':True})
    assert resumed['session']['id'] == saved['id'] and not resumed.get('initialization_error')
    assert not resumed['initializing']
    assert any(item['name'] == 'developer' and item['success'] for item in resumed['extension_results'])
    assert target.read_text() == edited, 'Restart changed the edited target before undo'
    store = Path(saved['undo_store'])
    assert snapshot_count(json.loads(store.read_text())) == 1
    edit('undo_edit')
    assert target.read_text() == original, 'Persisted undo did not restore the original file'
    assert snapshot_count(json.loads(store.read_text())) == 0, 'Successful undo did not consume its snapshot'
    outcome = {'same_session_resumed':True,'developer_loaded':True,'persisted_snapshot_loaded':True,
        'editor_undo_succeeded':True,'original_file_restored':True,'snapshot_consumed':True}
else:
    raise AssertionError('Unknown undo fixture phase')
outcome.update(owner_euid=os.geteuid(),undo_directory_mode=oct(undo_directory.stat().st_mode & 511),
    undo_file_mode=oct(store.stat().st_mode & 511),undo_file_uid=store.stat().st_uid)
print(json.dumps(outcome))
"""
UNDO_OBSERVER_PROBE = """
import json, os
from pathlib import Path
assert os.geteuid() == 10002
saved = json.loads(Path('/home/br/fixture.json').read_text())
directory = Path('/home/br/app-root/config/developer_undo')
result = {'observer_euid':os.geteuid(),'directory_list_denied':False,'snapshot_open_denied':False,
    'has_daemon_secret':'BIOROUTER_SERVER__SECRET_KEY' in os.environ,
    'public_control_read':Path('/home/br/public-control.txt').read_text() == 'SYNTHETIC_PUBLIC_CONTROL',
    'ancestors_traversable':all(os.access(path,os.X_OK) for path in
        ('/home','/home/br','/home/br/app-root','/home/br/app-root/config'))}
try:
    list(directory.iterdir())
except PermissionError:
    result['directory_list_denied'] = True
try:
    with Path(saved['undo_store']).open('rb') as snapshot:
        snapshot.read(1)
except PermissionError:
    result['snapshot_open_denied'] = True
print(json.dumps(result))
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
LEGACY_PREPARE_PROBE = """
import json, os, sqlite3
from contextlib import closing
from pathlib import Path
assert os.geteuid() == 10001
database = Path('/home/br/app-root/data/sessions/sessions.db')
with closing(sqlite3.connect(database)) as connection:
    journal_mode = connection.execute('PRAGMA journal_mode').fetchone()[0]
    checkpoint = connection.execute('PRAGMA wal_checkpoint(TRUNCATE)').fetchone()
    assert checkpoint[0] == 0 and checkpoint[1] == checkpoint[2], 'Legacy fixture checkpoint was incomplete'
for path in database.parent.iterdir():
    if path.name in ('sessions.db', 'sessions.db-wal', 'sessions.db-shm'):
        path.chmod(0o644)
print(json.dumps({'checkpoint':checkpoint,'source_journal_mode':journal_mode}))
"""
LEGACY_OBSERVER_PROBE = """
import json, os, shutil, sqlite3, tempfile
from contextlib import closing
from pathlib import Path
assert os.geteuid() == 10002
database = Path('/home/br/app-root/data/sessions/sessions.db')
result = {'observer_euid':os.geteuid(),'positive_control_method':'copied_readable_database',
    'raw_file_read':False,'copy_succeeded':False,'copied_sqlite_read':False,
    'synthetic_marker_read':False,'has_daemon_secret':'BIOROUTER_SERVER__SECRET_KEY' in os.environ}
try:
    with database.open('rb') as source:
        result['raw_file_read'] = source.read(16) == b'SQLite format 3\\0'
    with tempfile.TemporaryDirectory(prefix='biorouter-legacy-readable-') as temporary:
        directory = Path(temporary)
        copied_database = directory / 'sessions.db'
        shutil.copyfile(database, copied_database)
        copied_database.chmod(0o600)
        result.update(copy_succeeded=True,copy_directory_mode=oct(directory.stat().st_mode & 511),
            copy_database_mode=oct(copied_database.stat().st_mode & 511))
        with closing(sqlite3.connect(copied_database)) as connection:
            names = [row[0] for row in connection.execute('SELECT name FROM sessions')]
        result['copied_sqlite_read'] = True
        result['synthetic_marker_read'] = 'SYNTHETIC_REBUILT_HISTORY' in names
except (OSError, sqlite3.Error) as error:
    result['copy_query_error_class'] = type(error).__name__
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


def record_undo_observer(result, case_id, observed):
    record_case(result, case_id, observed,
                {"observer_euid": observed["observer_euid"] == 10002,
                 "public_control_read": observed["public_control_read"],
                 "parents_traversable": observed["ancestors_traversable"],
                 "undo_directory_denied": observed["directory_list_denied"],
                 "known_snapshot_path_denied": observed["snapshot_open_denied"],
                 "no_daemon_secret": not observed["has_daemon_secret"]})


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
               "limitation": "Synthetic rebuilt-daemon SQLite/Developer undo access and restart tests; not same-user or administrator isolation, a release build, or a HIPAA assessment."}
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
            result["phase"] = "fresh_persisted_undo"
            undo_fresh = json.loads(inside(container, OWNER, ["python3", "-c", UNDO_PROBE, "write"], secret=secret).stdout)
            inside(container, OWNER, ["chmod", "755", ROOT, ROOT + "/config"])
            undo_observer = json.loads(inside(container, OBSERVER, ["python3", "-c", UNDO_OBSERVER_PROBE]).stdout)
            record_case(result, "owner_fresh_undo", undo_fresh,
                        {"owner_euid": undo_fresh["owner_euid"] == 10001,
                         "real_editor_write": undo_fresh["editor_write_succeeded"],
                         "edited_file_matches": undo_fresh["edited_file_matches"],
                         "original_snapshot_persisted": undo_fresh["persisted_original_snapshot"],
                         "one_original_snapshot": undo_fresh["original_snapshot_count"] == 1,
                         "undo_directory_0700": undo_fresh["undo_directory_mode"] == "0o700",
                         "undo_file_0600": undo_fresh["undo_file_mode"] == "0o600",
                         "undo_file_owner": undo_fresh["undo_file_uid"] == 10001})
            record_undo_observer(result, "observer_fresh_undo", undo_observer)
            capture_daemon_logs(output, f"repetition-{repetition}-initial", container, secret)
            stop_daemon(container)
            result["phase"] = "legacy_positive_control"
            inside(container, OWNER, ["chmod", "755", ROOT + "/data/sessions"])
            legacy_prepared = json.loads(inside(container, OWNER, ["python3", "-c", LEGACY_PREPARE_PROBE]).stdout)
            outsider_legacy = json.loads(inside(container, OBSERVER, ["python3", "-c", LEGACY_OBSERVER_PROBE]).stdout)
            outsider_legacy["owner_preparation"] = legacy_prepared
            record_case(result, "observer_legacy_readable_control", outsider_legacy,
                        {"raw_open_allowed": outsider_legacy["raw_file_read"],
                         "observer_private_copy": outsider_legacy["copy_succeeded"]
                         and outsider_legacy.get("copy_directory_mode") == "0o700"
                         and outsider_legacy.get("copy_database_mode") == "0o600",
                         "copied_sqlite_query_allowed": outsider_legacy["copied_sqlite_read"],
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
            result["phase"] = "restore_persisted_undo"
            undo_reopened = json.loads(inside(container, OWNER, ["python3", "-c", UNDO_PROBE, "restore"], secret=secret).stdout)
            undo_observer = json.loads(inside(container, OBSERVER, ["python3", "-c", UNDO_OBSERVER_PROBE]).stdout)
            record_case(result, "owner_persisted_undo", undo_reopened,
                        {"owner_euid": undo_reopened["owner_euid"] == 10001,
                         "same_session_resumed": undo_reopened["same_session_resumed"],
                         "normal_developer_loaded": undo_reopened["developer_loaded"],
                         "persisted_snapshot_loaded": undo_reopened["persisted_snapshot_loaded"],
                         "real_editor_undo": undo_reopened["editor_undo_succeeded"],
                         "original_file_restored": undo_reopened["original_file_restored"],
                         "successful_snapshot_consumed": undo_reopened["snapshot_consumed"],
                         "undo_directory_0700": undo_reopened["undo_directory_mode"] == "0o700",
                         "undo_file_0600": undo_reopened["undo_file_mode"] == "0o600",
                         "undo_file_owner": undo_reopened["undo_file_uid"] == 10001})
            record_undo_observer(result, "observer_reopened_undo", undo_observer)
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
