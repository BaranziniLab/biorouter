#!/usr/bin/env python3
"""Separate-UID probes against actual release-created state, using synthetic data."""
import argparse
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import time
import urllib.request

SECRET = "ba11ab1e00000000000000000000000000000000000000000000000000000001"
ROOT = Path("/home/br/state")
SECRET_PATH = ROOT / "config/secrets.yaml"
DB_PATH = ROOT / "data/sessions/sessions.db"
CONFIG_PATH = ROOT / "config/config.yaml"
CASE_PATH = Path(__file__).with_name("environment_local_cases.json")


def api(method, path, body):
    req = urllib.request.Request(
        "http://127.0.0.1:3000" + path,
        data=json.dumps(body).encode(),
        method=method,
        headers={"x-secret-key": SECRET, "Content-Type": "application/json"},
    )
    with urllib.request.urlopen(req, timeout=8) as response:
        data = response.read()
        return json.loads(data) if data else None


def seed(repetition):
    marker = f"BRSEC-SYNTHETIC-LOCAL-SESSION-REP{repetition}"
    secret = f"BRSEC-SYNTHETIC-LOCAL-SECRET-REP{repetition}"
    api("POST", "/config/upsert", {"key": "BRSEC_SYNTHETIC_LOCAL_SECRET", "value": secret, "is_secret": True})
    session = api("POST", "/agent/start", {"working_dir": "/home/br"})
    api("PUT", f"/sessions/{session['id']}/name", {"name": marker})
    Path("/tmp/brsec-local-fixture.json").write_text(json.dumps({"session_marker": marker, "secret_marker": secret, "session_id": session["id"]}))
    print(json.dumps({"seeded": True, "session_id": session["id"], "secret_canary_only": True}))


def operation(name, fixture):
    if name == "secret_read":
        return {"canary_read": fixture["secret_marker"] in SECRET_PATH.read_text(), "path": str(SECRET_PATH)}
    if name == "session_read":
        with sqlite3.connect(f"file:{DB_PATH}?mode=ro", uri=True, timeout=2) as database:
            rows = database.execute("SELECT name FROM sessions WHERE id = ?", (fixture["session_id"],)).fetchall()
        return {"canary_read": any(fixture["session_marker"] in row[0] for row in rows), "path": str(DB_PATH)}
    if name == "config_write":
        with CONFIG_PATH.open("ab") as output:
            output.write(b"\nBRSEC_UNAUTHORIZED_LOCAL_WRITE: true\n")
        return {"unauthorized_write": True, "path": str(CONFIG_PATH)}
    if name == "daemon_environ":
        pids = [p for p in Path("/proc").iterdir() if p.name.isdigit()]
        for proc in pids:
            try:
                command = (proc / "comm").read_text().strip()
            except (OSError, PermissionError):
                continue
            if command == "biorouterd":
                data = (proc / "environ").read_bytes()
                return {"canary_read": SECRET.encode() in data, "path": str(proc / "environ")}
        raise RuntimeError("Daemon process not located")
    if name == "workspace_write":
        path = Path("/home/br/workspace-control.txt")
        path.write_text("BRSEC-SYNTHETIC-WORKSPACE-CONTROL")
        return {"write_readback": path.read_text() == "BRSEC-SYNTHETIC-WORKSPACE-CONTROL"}
    if name == "release_write":
        with Path("/opt/biorouter/biorouterd").open("ab") as output:
            output.write(b"BRSEC")
        return {"unauthorized_write": True}
    raise ValueError(name)


def observe(repetition):
    manifest = json.loads(CASE_PATH.read_text())
    manifest_hash = hashlib.sha256(CASE_PATH.read_bytes()).hexdigest()
    actor = "owner_uid10001" if os.getuid() == 10001 else "outsider_uid10002"
    fixture = json.loads(Path("/tmp/brsec-local-fixture.json").read_text())
    for case in manifest["cases"]:
        if case["actor"] != actor:
            continue
        started = time.monotonic()
        record = {"schema_version": 1, "run_id": f"brsec-local-repetition-{repetition}", "case_id": case["case_id"], "family": case["case_id"][0], "case_type": case["case_type"], "actor": actor, "actor_has_daemon_secret_env": "BIOROUTER_SERVER__SECRET_KEY" in os.environ, "repetition": repetition, "version": "1.93.4", "commit": "b824e2c3e802e243804a150aa7c349fa6ce6a8b6", "configuration": manifest["configuration"], "fixture_sha256": hashlib.sha256(json.dumps(fixture, sort_keys=True).encode()).hexdigest(), "case_manifest_sha256": manifest_hash, "expected_protected_effect": case["expected"], "utc_start": dt.datetime.now(dt.timezone.utc).isoformat(), "limitation": manifest["limitations"]}
        try:
            result = operation(case["operation"], fixture)
            record["observed_effect"] = result
            success = any(result.get(key, False) for key in ("canary_read", "unauthorized_write", "write_readback"))
            record["outcome"] = ("allowed_control" if success else "blocked_control") if case["case_type"] == "control" else ("attack_succeeded" if success else "inconclusive")
        except PermissionError as error:
            record["observed_effect"] = {"errno": error.errno, "error": error.strerror, "path": str(error.filename)}
            record["outcome"] = "blocked_control" if case["case_type"] == "control" else "blocked"
        except OSError as error:
            record["observed_effect"] = {"errno": error.errno, "error": str(error)}
            record["outcome"] = "blocked" if error.errno == 30 and case["case_type"] == "attack" else "error"
        except Exception as error:
            record["observed_effect"] = {"error": str(error), "type": type(error).__name__}
            record["outcome"] = "error"
        record["duration_ms"] = round((time.monotonic() - started) * 1000, 3)
        print(json.dumps(record), flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=("seed", "observe"))
    parser.add_argument("--repetition", type=int, choices=(1, 2, 3), required=True)
    args = parser.parse_args()
    seed(args.repetition) if args.command == "seed" else observe(args.repetition)
