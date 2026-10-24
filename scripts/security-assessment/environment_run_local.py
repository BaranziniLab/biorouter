#!/usr/bin/env python3
"""Run the frozen separate-UID cases in three fresh, owned containers."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

HERE = Path(__file__).resolve().parent
PERMISSIONS = '''from pathlib import Path
import json
paths = [Path('/home/br'), Path('/home/br/state'), Path('/home/br/state/data'), Path('/home/br/state/data/sessions'), Path('/home/br/state/data/sessions/sessions.db'), Path('/home/br/state/data/sessions/sessions.db-wal'), Path('/home/br/state/data/sessions/sessions.db-shm'), Path('/home/br/state/config'), Path('/home/br/state/config/secrets.yaml')]
print(json.dumps([{'path': str(p), 'mode': oct(p.stat().st_mode & 511), 'uid': p.stat().st_uid, 'gid': p.stat().st_gid} for p in paths if p.exists()], indent=2))
'''


def run(*args):
    return subprocess.run(args, capture_output=True, text=True, check=True).stdout


def main(runtime):
    os.environ["BRSEC_RUNTIME"] = str(runtime)
    events = []
    for repetition in (1, 2, 3):
        output = runtime / f"evidence/local/repetition-{repetition}"
        output.mkdir(parents=True, exist_ok=True)
        run("bash", str(HERE / "environment.sh"), "stop")
        (output / "start.log").write_text(run("bash", str(HERE / "environment.sh"), "start", str(repetition)))
        for attempt in range(30):
            ready = subprocess.run(["docker", "exec", "--user", "10002:10002", "brsec-target", "python3", "-c", "import urllib.request; assert urllib.request.urlopen('http://127.0.0.1:3000/status',timeout=1).read()==b'ok'"], capture_output=True)
            if ready.returncode == 0:
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("Daemon did not become ready")
        (output / "runtime.json").write_text(run("bash", str(HERE / "environment.sh"), "inspect"))
        (output / "seed.json").write_text(run("docker", "exec", "--user", "10001:10001", "brsec-target", "python3", "/assessment/harness/environment_local_probe.py", "seed", "--repetition", str(repetition)))
        for role, uid in (("owner", "10001:10001"), ("outsider", "10002:10002")):
            records = run("docker", "exec", "--user", uid, "brsec-target", "python3", "/assessment/harness/environment_local_probe.py", "observe", "--repetition", str(repetition))
            (output / f"{role}.jsonl").write_text(records)
            events.extend(json.loads(row) for row in records.splitlines())
        (output / "permissions.json").write_text(run("docker", "exec", "--user", "10001:10001", "brsec-target", "python3", "-c", PERMISSIONS))
        (output / "daemon.stderr").write_text(run("docker", "exec", "--user", "10001:10001", "brsec-target", "cat", "/home/br/daemon.stderr"))
        print(json.dumps({"repetition": repetition, "outcomes": {outcome: sum(record["outcome"] == outcome for record in events if record["repetition"] == repetition) for outcome in sorted({record["outcome"] for record in events})}}), flush=True)
    run("bash", str(HERE / "environment.sh"), "stop")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path, default=Path("/Users/wgu/biorouter-runs/security-assessment-2026-10"))
    main(parser.parse_args().runtime)
