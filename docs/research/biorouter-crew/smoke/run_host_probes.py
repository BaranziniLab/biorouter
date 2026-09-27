#!/usr/bin/env python3
"""Run small synthetic primitives and byte transport probes on supplied SSH hosts."""

import datetime
import hashlib
import json
import pathlib
import subprocess
import sys
import time


SSH_OPTIONS = [
    "-T", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=yes",
    "-o", "ConnectTimeout=15", "-o", "ConnectionAttempts=1",
    "-o", "ForwardAgent=no", "-o", "ServerAliveInterval=10",
    "-o", "ServerAliveCountMax=2",
]


def main():
    here = pathlib.Path(__file__).resolve().parent
    hosts = sys.argv[1:] or ["wagu@narrows-login.sdsc.edu", "wanjun@leo.ucsf.edu"]
    probe = (here / "host_probe.py").read_bytes()
    results = []
    for host in hosts:
        start = time.monotonic()
        command = ["ssh", *SSH_OPTIONS, host, "python3 -"]
        run = subprocess.run(command, input=probe, capture_output=True, timeout=45)
        entry = {"target": host, "returncode": run.returncode,
                 "elapsed_seconds": round(time.monotonic() - start, 3)}
        try:
            entry["primitives"] = json.loads(run.stdout.decode())
        except (ValueError, UnicodeError):
            entry["stdout"] = run.stdout.decode(errors="replace")[-2000:]
        if run.stderr:
            entry["stderr"] = run.stderr.decode(errors="replace")[-2000:]
        payload = bytes(range(256)) * 4096
        echo = "python3 -c 'import sys; data=sys.stdin.buffer.read(1048577); sys.stdout.buffer.write(data)'"
        transport = subprocess.run(["ssh", *SSH_OPTIONS, host, echo], input=payload,
                                   capture_output=True, timeout=45)
        entry["ssh_stdio_binary_roundtrip"] = {
            "returncode": transport.returncode,
            "bytes_sent": len(payload), "bytes_received": len(transport.stdout),
            "sha256_match": hashlib.sha256(payload).digest() == hashlib.sha256(transport.stdout).digest(),
        }
        # Let the SFTP client wait for the reply before closing stdin. A raw INIT
        # followed immediately by EOF can make the server exit without replying.
        sftp = subprocess.run(["sftp", "-q", "-b", "-", *SSH_OPTIONS[1:], host],
                              input=b"pwd\nquit\n", capture_output=True, timeout=30)
        entry["sftp_subsystem"] = {
            "pwd_command_completed": sftp.returncode == 0 and b"Remote working directory:" in sftp.stdout,
            "returncode": sftp.returncode,
        }
        if sftp.returncode:
            entry["sftp_subsystem"]["stderr"] = sftp.stderr.decode(errors="replace")[-2000:]
        results.append(entry)
        print(json.dumps(entry, sort_keys=True), flush=True)
    report = {"tested_at_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
              "probe_scope": "temporary synthetic fixtures; no deployment or PHI",
              "results": results}
    output = here.parent / "institutional-host-smoke.json"
    output.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    return 0 if all(e["returncode"] == 0 and e.get("primitives", {}).get("passed") and
                    e["ssh_stdio_binary_roundtrip"]["returncode"] == 0 and
                    e["ssh_stdio_binary_roundtrip"]["sha256_match"] and
                    e["sftp_subsystem"]["pwd_command_completed"] for e in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
